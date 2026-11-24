use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::{
    ParseError, ast::Statement, module_importer::ModuleImporter, parser::parse, span::Span,
};

use codespan_reporting::files::{Error as FilesError, Files, SimpleFile};
use compact_str::{CompactString, ToCompactString};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModulePath(pub Vec<CompactString>);

impl std::fmt::Display for ModulePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", itertools::join(self.0.iter().cloned(), "::"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModulePathBorrowed<'a>(pub Vec<&'a str>);

#[derive(Debug, Clone)]
pub enum CodeSource {
    /// User input from the command line or a REPL
    Text,

    /// For internal use, e.g. when executing special statements (like "use prelude")
    /// during startup.
    Internal,

    /// A file that has been read in
    File(PathBuf),

    /// A module that has been imported
    Module(ModulePath, Option<PathBuf>),
}

#[derive(Error, Clone, Debug)]
pub enum ResolverError {
    #[error("Unknown module '{1}'.")]
    UnknownModule(Span, ModulePath),

    #[error("{}", .0.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))]
    ParseErrors(Vec<ParseError>),

    #[error(
        "Definitions and imports can't be forgotten, so only expressions are interpreted here."
    )]
    Definition,
}

type Result<T> = std::result::Result<T, ResolverError>;

/// codespan's `SimpleFiles`, which can't be truncated.
#[derive(Clone, Default)]
pub struct SourceFiles(Vec<SimpleFile<String, String>>);

impl SourceFiles {
    fn add(&mut self, name: String, source: String) -> usize {
        self.0.push(SimpleFile::new(name, source));
        self.0.len() - 1
    }

    pub fn get(
        &self,
        file_id: usize,
    ) -> std::result::Result<&SimpleFile<String, String>, FilesError> {
        self.0.get(file_id).ok_or(FilesError::FileMissing)
    }
}

impl<'a> Files<'a> for SourceFiles {
    type FileId = usize;
    type Name = String;
    type Source = &'a str;

    fn name(&self, file_id: usize) -> std::result::Result<String, FilesError> {
        Ok(self.get(file_id)?.name().clone())
    }

    fn source(&self, file_id: usize) -> std::result::Result<&str, FilesError> {
        Ok(self.get(file_id)?.source())
    }

    fn line_index(
        &self,
        file_id: usize,
        byte_index: usize,
    ) -> std::result::Result<usize, FilesError> {
        self.get(file_id)?.line_index((), byte_index)
    }

    fn line_range(
        &self,
        file_id: usize,
        line_index: usize,
    ) -> std::result::Result<std::ops::Range<usize>, FilesError> {
        self.get(file_id)?.line_range((), line_index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResolverLengths {
    files: usize,
    text_code_source_count: usize,
    internal_code_source_count: usize,
    imported_modules: usize,
}

#[derive(Clone)]
pub struct Resolver {
    importer: Arc<dyn ModuleImporter>,
    pub files: SourceFiles,
    text_code_source_count: usize,
    internal_code_source_count: usize,
    pub imported_modules: Vec<ModulePath>,
    codesources: HashMap<usize, CodeSource>,
}

impl Resolver {
    pub(crate) fn new(importer: impl ModuleImporter + 'static) -> Self {
        Self {
            importer: Arc::new(importer),
            files: SourceFiles::default(),
            text_code_source_count: 0,
            internal_code_source_count: 0,
            imported_modules: vec![],
            codesources: HashMap::new(),
        }
    }

    pub fn add_code_source(&mut self, code_source: CodeSource, content: &str) -> usize {
        let code_source_name = match &code_source {
            CodeSource::Text => {
                self.text_code_source_count += 1;
                format!("<input:{}>", self.text_code_source_count)
            }
            CodeSource::Internal => {
                self.internal_code_source_count += 1;
                format!("<internal:{}>", self.internal_code_source_count)
            }
            CodeSource::File(path) => format!("File {}", path.to_string_lossy()),
            CodeSource::Module(module_path, path) => format!(
                "Module '{module_path}', File {path}",
                module_path = itertools::join(module_path.0.iter(), "::"),
                path = path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or("?".into()),
            ),
        };

        let id = self.files.add(code_source_name, content.to_string());
        self.codesources.insert(id, code_source);

        id
    }

    pub(crate) fn lengths(&self) -> ResolverLengths {
        ResolverLengths {
            files: self.files.0.len(),
            text_code_source_count: self.text_code_source_count,
            internal_code_source_count: self.internal_code_source_count,
            imported_modules: self.imported_modules.len(),
        }
    }

    /// Forgets the code sources and imports added since `lengths` were taken.
    pub(crate) fn truncate(&mut self, lengths: ResolverLengths) {
        for id in lengths.files..self.files.0.len() {
            self.codesources.remove(&id);
        }
        self.files.0.truncate(lengths.files);
        self.text_code_source_count = lengths.text_code_source_count;
        self.internal_code_source_count = lengths.internal_code_source_count;
        self.imported_modules.truncate(lengths.imported_modules);
    }

    pub fn get_code_source(&self, id: usize) -> CodeSource {
        self.codesources.get(&id).cloned().unwrap()
    }

    fn parse<'a>(&self, code: &'a str, code_source_id: usize) -> Result<Vec<Statement<'a>>> {
        parse(code, code_source_id).map_err(|e| ResolverError::ParseErrors(e.1))
    }

    fn inlining_pass<'a>(&mut self, program: &[Statement<'a>]) -> Result<Vec<Statement<'a>>> {
        let mut new_program = vec![];

        for statement in program {
            match statement {
                Statement::ModuleImport(span, ModulePathBorrowed(module_path)) => {
                    if !self.imported_modules.iter().any(|m| &m.0 == module_path) {
                        let module_path =
                            ModulePath(module_path.iter().map(|s| s.to_compact_string()).collect());
                        if let Some((code, filesystem_path)) = self.importer.import(&module_path) {
                            let code: &'static str = Box::leak(code.to_string().into_boxed_str());
                            self.imported_modules.push(module_path.clone());
                            let code_source_id = self.add_code_source(
                                CodeSource::Module(module_path, filesystem_path),
                                code,
                            );

                            let imported_program = self.parse(code, code_source_id)?;
                            let inlined_program = self.inlining_pass(&imported_program)?;
                            for statement in inlined_program {
                                new_program.push(statement);
                            }
                        } else {
                            return Err(ResolverError::UnknownModule(*span, module_path.clone()));
                        }
                    }
                }
                statement => new_program.push(statement.clone()),
            }
        }

        Ok(new_program)
    }

    pub fn resolve<'a>(
        &mut self,
        code: &'a str,
        code_source: CodeSource,
    ) -> Result<Vec<Statement<'a>>> {
        let code_source_id = self.add_code_source(code_source, code);
        let statements = self.parse(code, code_source_id)?;

        self.inlining_pass(&statements)
    }

    pub fn get_importer(&self) -> &dyn ModuleImporter {
        self.importer.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ast::{DefineVariable, Expression},
        number::Number,
    };

    use super::*;

    struct TestImporter {}

    impl ModuleImporter for TestImporter {
        fn import(&self, path: &ModulePath) -> Option<(String, Option<PathBuf>)> {
            match path {
                ModulePath(p) if p == &["foo", "bar"] => Some(("use foo::baz".into(), None)),
                ModulePath(p) if p == &["foo", "baz"] => Some(("let a = 1".into(), None)),
                // ----
                ModulePath(p) if p == &["mod_a"] => Some(("use mod_b".into(), None)),
                ModulePath(p) if p == &["mod_b"] => Some(("use mod_c\n let x = y".into(), None)),
                ModulePath(p) if p == &["mod_c"] => Some(("let y = 1".into(), None)),
                // ----
                ModulePath(p) if p == &["cycle_a"] => Some(("use cycle_b".into(), None)),
                ModulePath(p) if p == &["cycle_b"] => Some(("use cycle_a".into(), None)),
                _ => None,
            }
        }

        fn list_modules(&self) -> Vec<ModulePath> {
            unimplemented!()
        }
    }

    #[test]
    fn resolver_basic_import() {
        use crate::ast::ReplaceSpans;

        let program = "
        use foo::bar
        a
        ";

        let importer = TestImporter {};

        let mut resolver = Resolver::new(importer);
        let program_inlined = resolver.resolve(program, CodeSource::Internal).unwrap();

        assert_eq!(
            &program_inlined.replace_spans(),
            &[
                Statement::DefineVariable(DefineVariable {
                    identifier_span: Span::dummy(),
                    identifier: "a",
                    expr: Expression::Scalar(Span::dummy(), Number::from_f64(1.0)),
                    type_annotation: None,
                    decorators: Vec::new(),
                }),
                Statement::Expression(Expression::Identifier(Span::dummy(), "a"))
            ]
        );
    }

    #[test]
    fn resolver_repeated_includes() {
        use crate::ast::ReplaceSpans;

        let program = "
        use foo::bar
        use foo::bar
        a
        ";

        let importer = TestImporter {};

        let mut resolver = Resolver::new(importer);
        let program_inlined = resolver.resolve(program, CodeSource::Internal).unwrap();

        assert_eq!(
            &program_inlined.replace_spans(),
            &[
                Statement::DefineVariable(DefineVariable {
                    identifier_span: Span::dummy(),
                    identifier: "a",
                    expr: Expression::Scalar(Span::dummy(), Number::from_f64(1.0)),
                    type_annotation: None,
                    decorators: Vec::new(),
                }),
                Statement::Expression(Expression::Identifier(Span::dummy(), "a"))
            ]
        );
    }

    #[test]
    fn resolver_depth_first_includes() {
        use crate::ast::ReplaceSpans;

        let program = "
        use mod_a
        use mod_c
        ";

        let importer = TestImporter {};

        let mut resolver = Resolver::new(importer);
        let program_inlined = resolver.resolve(program, CodeSource::Internal).unwrap();

        assert_eq!(
            &program_inlined.replace_spans(),
            &[
                Statement::DefineVariable(DefineVariable {
                    identifier_span: Span::dummy(),
                    identifier: "y",
                    expr: Expression::Scalar(Span::dummy(), Number::from_f64(1.0)),
                    type_annotation: None,
                    decorators: Vec::new(),
                }),
                Statement::DefineVariable(DefineVariable {
                    identifier_span: Span::dummy(),
                    identifier: "x",
                    expr: Expression::Identifier(Span::dummy(), "y"),
                    type_annotation: None,
                    decorators: Vec::new(),
                }),
            ]
        );
    }

    #[test]
    fn resolved_cyclic_imports() {
        let program = "
        use cycle_a
        ";

        let importer = TestImporter {};

        let mut resolver = Resolver::new(importer);
        let program_inlined = resolver.resolve(program, CodeSource::Internal).unwrap();

        assert_eq!(&program_inlined, &[]);
    }
}
