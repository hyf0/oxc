use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk::walk_call_expression};
use oxc_semantic::Scoping;

use crate::state::PassDirty;

/// Walks AST subtrees being dropped or replaced, collecting
/// `IdentifierReference`s and direct `eval(...)` calls into the per-pass
/// `PassDirty` accumulator.
///
/// Mark-only semantics: every resolved reference found in a dropped subtree is
/// ADDED to `dirty.dead_refs`; every direct eval call sets
/// `dirty.eval_dropped = true`. Unresolved references are not tracked (see
/// `visit_identifier_reference`).
///
/// There is deliberately no "resurrect" walk over replacement values: a
/// `ReferenceId` marked dead can never reappear in a replacement. Subtrees
/// moved out of the old slot into the new value leave id-less `TakeIn` dummies
/// behind, so the dead-walk never sees their ids; and the one site that used
/// to copy `ReferenceId`s into its replacement
/// (`substitute_is_object_and_not_null`) now mints fresh references instead.
pub struct DropDiff<'a, 's> {
    dirty: &'s mut PassDirty<'a>,
    scoping: &'s Scoping,
}

impl<'a, 's> DropDiff<'a, 's> {
    pub(crate) fn new(dirty: &'s mut PassDirty<'a>, scoping: &'s Scoping) -> Self {
        Self { dirty, scoping }
    }

    pub(crate) fn walk_old_expression(mut self, expr: &Expression<'a>) -> Self {
        self.visit_expression(expr);
        self
    }

    pub(crate) fn walk_old_statement(mut self, stmt: &Statement<'a>) -> Self {
        self.visit_statement(stmt);
        self
    }

    pub(crate) fn walk_old_assignment_target_property(
        mut self,
        prop: &AssignmentTargetProperty<'a>,
    ) -> Self {
        self.visit_assignment_target_property(prop);
        self
    }

    pub(crate) fn walk_old_property_key(mut self, key: &PropertyKey<'a>) -> Self {
        self.visit_property_key(key);
        self
    }

    pub(crate) fn walk_old_for_statement_left(mut self, lhs: &ForStatementLeft<'a>) -> Self {
        self.visit_for_statement_left(lhs);
        self
    }

    pub(crate) fn walk_old_class_element(mut self, element: &ClassElement<'a>) -> Self {
        self.visit_class_element(element);
        self
    }

    /// Walks the whole declarator — binding pattern, TS type annotation, and
    /// init. Type annotations can contain references (e.g. computed keys in a
    /// type literal: `const r: { [sym]: string } = ...`), so walking only the
    /// init leaks them.
    pub(crate) fn walk_old_variable_declarator(mut self, decl: &VariableDeclarator<'a>) -> Self {
        self.visit_variable_declarator(decl);
        self
    }
}

impl<'a> Visit<'a> for DropDiff<'a, '_> {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        // Freshly built `IdentifierReference` nodes (e.g. created via
        // `ast.identifier_reference(...)` or as a `TakeIn` dummy left in place
        // by `take_in`) have no `reference_id` yet. Such nodes carry no
        // semantic state to mark dead, so skip them.
        let Some(reference_id) = it.reference_id.get() else { return };
        let resolved = self.scoping.get_reference(reference_id).symbol_id().is_some();
        if resolved {
            let idx = reference_id.index();
            // References minted mid-pass (fresh idents from substitutions) have
            // indices beyond the bitset's capacity (sized at `enter_program`).
            // They cannot have been alive when the pass began, and the retain
            // guard already treats `idx >= capacity` as live — skip marking
            // instead of panicking. This is a legal flow: a fresh ident minted
            // by one optimization in pass N can be dropped later in the same
            // pass by another, so no `debug_assert!` in the else branch. The
            // skip is conservative: the reference stays in its symbol's list
            // until callers rebuild scoping (a missed optimization, never a
            // correctness issue).
            if idx < self.dirty.dead_refs.capacity() {
                self.dirty.dead_refs.set_bit(idx);
            }
        }
        // Unresolved references (no `symbol_id`) are intentionally untracked:
        // `root_unresolved_references` is not consumed by any in-loop
        // optimization and the compressor's `Scoping` is never read back by a
        // consumer (callers rebuild it), so pruning it would be dead work.
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if !it.optional
            && let Some(ident) = it.callee.get_identifier_reference()
            && ident.name == "eval"
        {
            self.dirty.eval_dropped = true;
        }
        // Recurse — eval may be nested inside another call's arguments.
        walk_call_expression(self, it);
    }
}
