//! The composition algebra: fragments, the ⊕ fold, and its laws.
//!
//! This is the executable form of the contract in the crate root docs. Everything
//! here is pure; the laws at the bottom are property-tested, and the fold is the
//! single operation that will subsume `HoistedResources`, `dedup_resource_blocks`
//! and the `MERGE_KEY_PREFIX` rename/fold machinery once the differential corpus
//! proves byte-identical output.

use crate::{Address, MergeClass, Scope, Span};
use std::collections::{BTreeMap, BTreeSet};

/// One grant edge: (member, role, canonicalized condition). Grants form a set —
/// their merge is union, and no conflict state exists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GrantEdge {
    pub member: String,
    pub role: String,
    /// Canonical rendering of the condition, empty when absent.
    pub condition: String,
    /// `"import-id"` declared on this binding, empty when absent. Not part of
    /// the binding's identity: the emitter reconciles an edge declared with
    /// and without an id into one resource, and refuses two different ids.
    pub import_id: String,
}

/// A resource body in canonical form. Canonical equality — not byte equality — is
/// what idempotence is defined over (formatting differences are not conflicts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Additive IAM-style body.
    Grant(BTreeSet<GrantEdge>),
    /// Everything else: canonicalized attribute value.
    Attrs(serde_yaml::Value),
}

/// An entity with provenance. ⊕ unions provenance, so every conflict can name all
/// of its origins.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub addr: Address,
    pub scope: Scope,
    pub body: Body,
    pub provenance: Vec<Span>,
    /// Structural position (enclosing folder labels, outermost first). Meaningful
    /// for `Scope::Node` types — a project must reference its parent folder at
    /// emission. Intrinsic-scope types ignore it; ⊕ keeps the first occurrence's.
    pub node_path: Vec<String>,
}

/// A conflict is a value, not an early error: merge is total, compilation fails
/// iff the folded result contains any ⊥ — which reports every conflict at once.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub addr: Address,
    /// All disagreeing bodies with their provenance.
    pub candidates: Vec<(Body, Vec<Span>)>,
}

/// The fold's codomain: per address, either a merged entity or an absorbed conflict.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    Ok(Entity),
    Bottom(Conflict),
}

/// A fragment's resource layer. (The parameter layer lives with the front-ends;
/// its priority semantics are in the crate root docs and `Priority`.)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fragment {
    pub entities: BTreeMap<Address, Entity>,
}

/// The type table: merge class + intrinsic scope per resource type. Schema-driven
/// in production (never name heuristics); explicit in tests.
pub trait TypeTable {
    fn merge_class(&self, tf_type: &str) -> MergeClass;
    fn scope(&self, tf_type: &str) -> Scope;
}

/// ⊕ on two slots of the same address.
fn merge_slot(class: MergeClass, a: Slot, b: Slot) -> Slot {
    match (a, b) {
        // ⊥ absorbs, accumulating provenance from the other side.
        (Slot::Bottom(mut c), Slot::Ok(e)) | (Slot::Ok(e), Slot::Bottom(mut c)) => {
            if !c.candidates.iter().any(|(body, _)| *body == e.body) {
                c.candidates.push((e.body, e.provenance));
            } else if let Some((_, spans)) =
                c.candidates.iter_mut().find(|(body, _)| *body == e.body)
            {
                spans.extend(e.provenance);
            }
            Slot::Bottom(c)
        }
        (Slot::Bottom(mut c1), Slot::Bottom(c2)) => {
            for (body, spans) in c2.candidates {
                if let Some((_, s)) = c1.candidates.iter_mut().find(|(b, _)| *b == body) {
                    s.extend(spans);
                } else {
                    c1.candidates.push((body, spans));
                }
            }
            Slot::Bottom(c1)
        }
        (Slot::Ok(mut ea), Slot::Ok(eb)) => match class {
            MergeClass::Grant => {
                // Union of edges — additive, conflict-free by construction.
                let (mut ga, gb) = match (ea.body, eb.body) {
                    (Body::Grant(a), Body::Grant(b)) => (a, b),
                    (a, b) => {
                        // Grant-typed address carrying non-grant bodies: conflict.
                        return Slot::Bottom(Conflict {
                            addr: ea.addr,
                            candidates: vec![(a, ea.provenance), (b, eb.provenance)],
                        });
                    }
                };
                ga.extend(gb);
                ea.body = Body::Grant(ga);
                ea.provenance.extend(eb.provenance);
                Slot::Ok(ea)
            }
            MergeClass::Entity | MergeClass::Tree => {
                // Flat lattice: canonical-equal is idempotent, different is ⊥.
                // (Tree types never reach the fold with equal addresses in the
                // front-end; treating them flatly here keeps the operator total.)
                if ea.body == eb.body {
                    ea.provenance.extend(eb.provenance);
                    Slot::Ok(ea)
                } else {
                    Slot::Bottom(Conflict {
                        addr: ea.addr,
                        candidates: vec![
                            (ea.body, ea.provenance),
                            (eb.body, eb.provenance),
                        ],
                    })
                }
            }
        },
    }
}

/// The folded result of composing any number of fragments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Folded {
    pub slots: BTreeMap<Address, Slot>,
}

impl Folded {
    pub fn conflicts(&self) -> Vec<&Conflict> {
        self.slots
            .values()
            .filter_map(|s| match s {
                Slot::Bottom(c) => Some(c),
                _ => None,
            })
            .collect()
    }
}

/// F ⊕ G, pointwise over addresses. Total; never fails.
pub fn compose(table: &dyn TypeTable, mut acc: Folded, frag: &Fragment) -> Folded {
    for (addr, entity) in &frag.entities {
        let class = table.merge_class(&addr.tf_type);
        let incoming = Slot::Ok(entity.clone());
        let slot = match acc.slots.remove(addr) {
            None => incoming,
            Some(existing) => merge_slot(class, existing, incoming),
        };
        acc.slots.insert(addr.clone(), slot);
    }
    acc
}

/// Fold a whole list of fragments. The laws below guarantee the result does not
/// depend on the order of the list (up to provenance multiset ordering).
pub fn fold(table: &dyn TypeTable, frags: &[Fragment]) -> Folded {
    frags.iter().fold(Folded::default(), |acc, f| compose(table, acc, f))
}

// ---------------------------------------------------------------------------
// Observational equivalence: what the laws are stated over. Provenance is a
// multiset (order irrelevant); slots must agree on bodies/conflict candidate sets.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) fn obs_eq(a: &Folded, b: &Folded) -> bool {
    if a.slots.len() != b.slots.len() {
        return false;
    }
    a.slots.iter().all(|(addr, sa)| match (sa, b.slots.get(addr)) {
        (Slot::Ok(ea), Some(Slot::Ok(eb))) => {
            ea.body == eb.body && sorted(&ea.provenance) == sorted(&eb.provenance)
        }
        (Slot::Bottom(ca), Some(Slot::Bottom(cb))) => {
            let mut xa: Vec<&Body> = ca.candidates.iter().map(|(b, _)| b).collect();
            let mut xb: Vec<&Body> = cb.candidates.iter().map(|(b, _)| b).collect();
            xa.sort_by_key(|b| format!("{:?}", b));
            xb.sort_by_key(|b| format!("{:?}", b));
            xa == xb
        }
        _ => false,
    })
}

#[cfg(test)]
fn sorted(spans: &[Span]) -> Vec<Span> {
    let mut v = spans.to_vec();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// Property tests: the laws, executable.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod laws {
    use super::*;
    use proptest::prelude::*;

    struct TestTable;
    impl TypeTable for TestTable {
        fn merge_class(&self, tf_type: &str) -> MergeClass {
            if tf_type.contains("iam_member") {
                MergeClass::Grant
            } else {
                MergeClass::Entity
            }
        }
        fn scope(&self, tf_type: &str) -> Scope {
            if tf_type.contains("organization") {
                Scope::Org
            } else {
                Scope::Node
            }
        }
    }

    prop_compose! {
        fn arb_fragment()(entries in proptest::collection::vec((0..4u8, 0..2u8, 0..100u32), 0..6))
            -> Fragment
        {
            let mut f = Fragment::default();
            for (label, kind, seed) in entries {
                let tf_type = if kind == 0 { "google_organization_iam_member" } else { "google_widget" };
                let addr = Address { tf_type: tf_type.into(), label: format!("l{}", label) };
                // deterministic body from seed via a tiny strategy sample
                let body = if kind == 0 {
                    let mut s = BTreeSet::new();
                    if seed % 3 != 0 { s.insert(GrantEdge{member: format!("m{}", seed % 2), role: format!("r{}", seed % 3), condition: String::new(), import_id: String::new()}); }
                    Body::Grant(s)
                } else {
                    Body::Attrs(serde_yaml::Value::Number(((seed % 3) as u64).into()))
                };
                let entity = Entity {
                    node_path: Vec::new(),
                    addr: addr.clone(),
                    scope: TestTable.scope(tf_type),
                    body,
                    provenance: vec![Span { file: format!("f{}", seed % 5), line: seed }],
                };
                // last-in-fragment wins within one fragment (a fragment is a map)
                f.entities.insert(addr, entity);
            }
            f
        }
    }

    proptest! {
        /// (F ⊕ G) ⊕ H ≈ F ⊕ (G ⊕ H)
        #[test]
        fn associativity(f in arb_fragment(), g in arb_fragment(), h in arb_fragment()) {
            let t = TestTable;
            let left = compose(&t, compose(&t, fold(&t, &[f.clone()]), &g), &h);
            let gh = fold(&t, &[g, h]);
            let mut right = fold(&t, &[f]);
            for (addr, slot) in gh.slots {
                let class = t.merge_class(&addr.tf_type);
                let merged = match right.slots.remove(&addr) {
                    None => slot,
                    Some(existing) => super::merge_slot(class, existing, slot),
                };
                right.slots.insert(addr, merged);
            }
            prop_assert!(obs_eq(&left, &right), "\nleft:  {:#?}\nright: {:#?}", left, right);
        }

        /// F ⊕ G ≈ G ⊕ F
        #[test]
        fn commutativity(f in arb_fragment(), g in arb_fragment()) {
            let t = TestTable;
            let ab = fold(&t, &[f.clone(), g.clone()]);
            let ba = fold(&t, &[g, f]);
            prop_assert!(obs_eq(&ab, &ba), "\nab: {:#?}\nba: {:#?}", ab, ba);
        }

        /// F ⊕ F ≈ F — deep-equal dedup is a law, not a special case.
        #[test]
        fn idempotence(f in arb_fragment()) {
            let t = TestTable;
            let once = fold(&t, &[f.clone()]);
            let twice = fold(&t, &[f.clone(), f]);
            // provenance doubles; bodies and conflict sets must not change
            prop_assert_eq!(once.slots.len(), twice.slots.len());
            for (addr, s1) in &once.slots {
                match (s1, &twice.slots[addr]) {
                    (Slot::Ok(a), Slot::Ok(b)) => prop_assert_eq!(&a.body, &b.body),
                    (Slot::Bottom(_), Slot::Bottom(_)) => {}
                    _ => prop_assert!(false, "slot kind changed under self-merge"),
                }
            }
        }

        /// F ⊕ ε ≈ F
        #[test]
        fn unit(f in arb_fragment()) {
            let t = TestTable;
            let folded = fold(&t, &[f.clone(), Fragment::default()]);
            prop_assert!(obs_eq(&folded, &fold(&t, &[f])));
        }

        /// Permuting the fragment list never changes the observable result —
        /// the theorem that retires "include order matters".
        #[test]
        fn order_free(mut frags in proptest::collection::vec(arb_fragment(), 0..5), seed in 0..1000u32) {
            let t = TestTable;
            let base = fold(&t, &frags);
            // deterministic shuffle
            let n = frags.len();
            if n > 1 {
                for i in 0..n {
                    let j = (seed as usize + i * 7) % n;
                    frags.swap(i, j);
                }
            }
            let shuffled = fold(&t, &frags);
            prop_assert!(obs_eq(&base, &shuffled));
        }

        /// ⊥ absorbs: once an address conflicts, further deep-equal contributions
        /// never un-conflict it.
        #[test]
        fn bottom_absorbs(f in arb_fragment(), g in arb_fragment(), h in arb_fragment()) {
            let t = TestTable;
            let fg = fold(&t, &[f.clone(), g.clone()]);
            let fgh = fold(&t, &[f, g, h]);
            for (addr, slot) in &fg.slots {
                if matches!(slot, Slot::Bottom(_)) {
                    prop_assert!(matches!(fgh.slots.get(addr), Some(Slot::Bottom(_))),
                        "conflict at {:?} disappeared after composing more", addr);
                }
            }
        }
    }
}
