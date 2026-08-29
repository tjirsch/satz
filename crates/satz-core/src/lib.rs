//! satz-core — the typed composition core.
//!
//! satz's composition semantics grew organically, one real failure at a time:
//! first-definition-wins parameter overrides, `!include-if` guards, hoisted scopes,
//! schema-driven cross-file key merging, the duplicate-address guard. This crate
//! re-founds them as one small algebra over a typed IR, so that the six mechanisms
//! become one fold plus theorems instead of six passes.
//!
//! # The algebra (contract)
//!
//! A fragment is a pair `F = (P, E)`: a parameter environment and an entity map.
//!
//! **Resource layer.** `E : Address -> Entity`. The merge on bodies is chosen by the
//! *type's* merge class (schema-driven, never name heuristics):
//!
//! - `Grant`  — union of canonical (member, role, condition) edges
//! - `Entity` — flat lattice: `m(x, x) = x`, `m(x, y) = ⊥{x, y}` when `x ≠ y`
//! - `Tree`   — no merge; folder/project compose by grafting at a path
//!
//! Composition `⊕` is pointwise merge on address maps. Laws, up to observational
//! equivalence (same emitted output or same conflict set):
//!
//! ```text
//! (F ⊕ G) ⊕ H ≈ F ⊕ (G ⊕ H)     associativity
//! F ⊕ G       ≈ G ⊕ F           commutativity
//! F ⊕ F       ≈ F               idempotence  — deep-equal dedup is this law
//! F ⊕ ε       ≈ F               unit — a skipped conditional include contributes ε
//! ⊥ ⊕ x       ≈ ⊥               conflicts absorb, accumulating provenance
//! ```
//!
//! Merge is **total**: a conflict is a value `⊥{addr, (v₁, span₁), (v₂, span₂)}`, not
//! an early error — compilation fails iff the folded result contains any ⊥, which
//! reports *all* conflicts at once, each with both origins.
//!
//! **Parameter layer.** Bindings are `(name -> (priority, value, span))` with
//! `Default < Set` (< `Force`, reserved for the subtractive-override channel).
//! Higher priority wins; equal priority with different canonical values is ⊥;
//! equal values are idempotent. The whole algebra is therefore order-free —
//! today's "override must be textually above the include" becomes a theorem about
//! depth-derived priorities, and simultaneously obsolete.
//!
//! **Scope is a typing judgment.** `cloud_identity_group : scope Customer`,
//! `organization_iam_member : scope Org`. Evaluation at a tree position computes
//! scope from the type; for intrinsic scopes the position is discarded into
//! provenance. *Relocation theorem*: fragments of intrinsic scope produce identical
//! output at any tree position — hoisting is this theorem, not a pass.
//!
//! # Delivery contract
//!
//! satz (the binary in this workspace) is the reference implementation. This
//! crate replaces its internals only behind a differential harness proving
//! byte-identical output over the full preset corpus. Every satz bugfix adds a
//! fixture here. Success metric for the swap: `HoistedResources`,
//! `dedup_resource_blocks` and the `MERGE_KEY_PREFIX` rename/fold machinery deleted,
//! replaced by one fold whose laws are the property tests in this crate.

pub mod algebra;
pub mod migrate;
pub mod pipeline;
pub mod satz;

/// A source location carried through every transform. Errors must be able to name
/// every contributing origin, so provenance is a growing list, not a single site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub file: String,
    pub line: u32,
}

/// The Terraform-level identity of a resource: one address, emitted once — a law,
/// not a guard.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address {
    pub tf_type: String,
    pub label: String,
}

/// Where a resource intrinsically lives. A property of the *type* (schema-driven);
/// syntactic position is only provenance for intrinsic scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Cloud Identity: `customers/<id>`
    Customer,
    /// Organization-level: `org_id = <id>`
    Org,
    /// Billing account
    Billing,
    /// Lives where it is written (folder/project tree position is meaningful)
    Node,
}

/// How two bodies of the same address combine — chosen by the type, never by name shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeClass {
    /// Additive: union of canonical grant edges. No conflict state exists.
    Grant,
    /// Flat lattice: identical is idempotent, different is a conflict.
    Entity,
    /// Structural: grafted at a path, never merged.
    Tree,
}

/// Parameter binding strength. Order matters: `Default < Set < Force`.
/// `Force` is reserved for the subtractive-override channel (private roadmap, Phase 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// A preset's overridable default.
    Default,
    /// The including document's binding.
    Set,
    /// Reserved: unconditional override / suppression.
    Force,
}

#[cfg(test)]
mod contract {
    use super::*;

    /// The priority order is the load-bearing fact of the parameter layer.
    #[test]
    fn priority_order_is_default_set_force() {
        assert!(Priority::Default < Priority::Set);
        assert!(Priority::Set < Priority::Force);
    }
}
