//! A Rust port of [`@codemirror/language`](https://codemirror.net/docs/ref/#language)
//! 6.13.1, on [`editor_state`].
//!
//! The port so far covers the indentation unit of `indent.ts`: the
//! [`INDENT_UNIT`] facet with [`indent_unit`], [`get_indent_unit`] and
//! [`indent_string`], which the editing commands of `@codemirror/commands`
//! use.
//!
//! Not yet ported (milestone M8): the syntax tree and its language
//! (`Language`, `syntaxTree`, the parse worker), indentation from the tree
//! and indent services (`IndentContext`, `getIndentation`,
//! `indentService`), highlighting, folding, bracket matching and stream
//! parsers.

mod indent;

pub use indent::{get_indent_unit, indent_string, indent_unit, INDENT_UNIT};
