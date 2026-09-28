//! C2 (task.md): "resolve `node.kind()`/field-name lookups to numeric ids
//! (`kind_id()`/`child_by_field_id`) instead of the current per-call string
//! comparison, keyed off `file.tree.language()` and built once per
//! `scan_file` call". Built once per `lower_file` call from that file's own
//! `file.tree.language()` -- a fresh table per call rather than a single
//! cached one, since the JS/TS family alone spans three distinct grammars
//! (JavaScript/TypeScript/Tsx), each its own `Language` with its own id
//! space; a table cached by `LanguageFamily` would conflate them.

use std::collections::HashMap;

use tree_sitter::Language;

/// Every kind-name literal `src/lower/java.rs`/`src/lower/jsts.rs` match on,
/// across both lowerings -- a name absent from one grammar (e.g. Java has no
/// `pair` node; JS/TS has no `constructor_body`) simply resolves to an empty
/// id set below, matching the old string comparison's own "never matches"
/// behaviour for that grammar.
pub(crate) const KIND_NAMES: &[&str] = &[
    "return_statement",
    "break_statement",
    "continue_statement",
    "throw_statement",
    "method_declaration",
    "constructor_declaration",
    "compact_constructor_declaration",
    "static_initializer",
    "lambda_expression",
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "arrow_function",
    "method_definition",
    "block",
    "constructor_body",
    "statement_block",
    "if_statement",
    "for_statement",
    "enhanced_for_statement",
    "while_statement",
    "do_statement",
    "for_in_statement",
    "switch_label",
    "switch_case",
    "switch_default",
    "switch_block_statement_group",
    "catch_clause",
    "ternary_expression",
    "binary_expression",
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
    "annotation_type_declaration",
    "class_body",
    "object_creation_expression",
    "enum_constant",
    "class",
    "abstract_class_declaration",
    "internal_module",
    "module",
    "line_comment",
    "block_comment",
    "comment",
    "variable_declarator",
    "pair",
    "assignment_expression",
    "type_alias_declaration",
    "default",
    "formal_parameters",
    "using",
    "string",
    "&",
    "program",
    "&&",
    "||",
    "??",
    "spread_parameter",
];

/// Every field-name literal both lowerings look up via
/// `child_by_field_name`/`child_by_field_id`.
pub(crate) const FIELD_NAMES: &[&str] = &[
    "body",
    "name",
    "parameters",
    "type",
    "dimensions",
    "operator",
    "key",
    "left",
];

/// One file's kind-name -> every numeric id table (`KIND_NAMES`), plus the
/// field-name -> numeric field id table (`FIELD_NAMES`), both built once
/// from that file's own `Language`. Every lookup after `build` returns is a
/// `u16` comparison, never a string one.
pub(crate) struct KindIds {
    kinds: HashMap<&'static str, Vec<u16>>,
    fields: HashMap<&'static str, Option<u16>>,
}

impl KindIds {
    /// Builds the table via `Language::id_for_node_kind` -- one id per name.
    ///
    /// NOTE (WS-10 red commit): this is deliberately the wrong
    /// implementation the spec's own "do not reuse" line warns about --
    /// `id_for_node_kind` returns exactly one id per name and cannot see an
    /// aliased symbol id sharing that name, so `kind_id_tables_cover_every_alias_of_each_kind`
    /// fails against it. The green commit immediately following replaces
    /// this with the alias-aware sweep over `0..node_kind_count()`.
    pub(crate) fn build(
        language: &Language,
        kind_names: &[&'static str],
        field_names: &[&'static str],
    ) -> KindIds {
        let kinds = kind_names
            .iter()
            .map(|name| {
                let ids = match language.id_for_node_kind(name, true) {
                    0 => Vec::new(),
                    id => vec![id],
                };
                (*name, ids)
            })
            .collect();
        let fields = field_names
            .iter()
            .map(|name| {
                let field_id = language.field_id_for_name(name).map(|id| id.get());
                (*name, field_id)
            })
            .collect();
        KindIds { kinds, fields }
    }

    /// Whether `id` is one of `name`'s ids in this language, aliases
    /// included -- the numeric replacement for `node.kind() == name`.
    pub(crate) fn is(&self, name: &str, id: u16) -> bool {
        self.kinds.get(name).is_some_and(|ids| ids.contains(&id))
    }

    /// `name`'s full id set -- used by the coverage test below, which needs
    /// the table itself rather than a single membership answer. Test-only:
    /// no production caller needs the whole set, only single-id membership
    /// (`is` above).
    #[cfg(test)]
    pub(crate) fn ids(&self, name: &str) -> &[u16] {
        self.kinds.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The numeric field id for `name` in this language, if the grammar
    /// defines it -- the numeric replacement for
    /// `node.child_by_field_name(name)`.
    pub(crate) fn field(&self, name: &str) -> Option<u16> {
        self.fields.get(name).copied().flatten()
    }
}
