# 0090 — the project admins' horizontal reach is Writer plus Project IAM Admin, not Owner, and the default follows GA

- **Status:** accepted
- **Date:** 2026-11-12

## Context

The security-group models S1 and S2 (`presets/security-group-models/`) grant the project admins
group `roles/owner` on the organisation. The group is the horizontal platform team: it operates in
every project of the organisation, including the projects business units create under their own
folders, while the business units hold vertical roles on their own resources. Owner on the
organisation is what gave that reach to every service in every project, present and future.

Owner is a legacy basic role. Privileged Access Manager (PAM) cannot grant it and an IAM condition
cannot bound it, so the group cannot join a time-boxed access model while it holds Owner. Google's
basic roles Admin, Writer and Reader can be granted by PAM and bounded by conditions; the console
cannot grant them, only the API and gcloud; and they are in Preview. The question was what
guarantees the project admins' reach once Owner goes.

What guarantees the reach is not Owner. A role granted on the organisation is inherited by every
folder and project under it, including a project created tomorrow, and nothing in the allow model
subtracts it (only an IAM deny policy, a Principal Access Boundary policy or a VPC Service Controls
perimeter can, none of which a folder-level role creates). `roles/resourcemanager.projectIamAdmin`
on the organisation, which the group holds, is `projects.setIamPolicy` on every project: a project
admin can grant any role on any project, to themselves or a colleague, with an audit-log entry and
without an approver. Owner adds every service's permissions without that step, plus project
deletion, billing assignment and service-account IAM, plus organisation-level powers nobody chose
for the group: creating custom roles, editing VPC Service Controls perimeters, creating PAM
entitlements. It does not include organisation or folder IAM, folder or project creation, org
policy or billing-account administration. `roles/admin` on the organisation does include
organisation and folder IAM. `roles/writer` includes neither, nor project deletion nor billing
assignment.

## Options

1. **Writer plus the key.** `roles/writer`, `roles/resourcemanager.projectIamAdmin`, and the named
   roles Owner gives beyond Writer: `roles/iam.serviceAccountAdmin` (in place of
   `serviceAccountCreator`), `roles/resourcemanager.projectDeleter`, `roles/billing.projectManager`.
   - Pros: the same reach at the project level, every service present and future; PAM can grant
     the set and a condition can bound it; the three organisation-level powers Owner carried fall
     away.
   - Cons: a basic role stays in the model, and it is Preview; the legacy convenience grants
     (Cloud Storage `projectOwners`, BigQuery special groups) stop applying.
2. **Named roles only plus the key.** A list of service admin roles; anything missing is a
   self-grant on the project, time-boxed by convention.
   - Pros: no basic role.
   - Cons: every unlisted service needs a self-grant first; the list is maintained forever; a
     self-made binding outlives the task unless somebody conditions it, so the standing power is
     moved behind a logged step, not reduced.
3. **`roles/admin` on the workload folder(s).** Owner-equivalent inside the workload tree only.
   - Pros: no organisation-level power at all inside the tree.
   - Cons: nothing outside the folder; `workload_folder_name` may be empty, which is the
     organisation itself, and then Admin lands on the organisation with organisation IAM in it.
4. **Keep Owner.** No change; the group never joins a time-boxed model.

When the library's default changes was a second question: at once, which puts a Preview role on
every estate at its next apply, or when Google marks the roles GA.

## Decision

Option 1, and the default follows GA. S1 and S2 1.4 make the project admins' roles the list param
`gcp_project_admins_roles`; its default is the list the models granted before, `roles/owner`
included, so the update moves nothing in a plan. An estate that wants the reach PAM-grantable binds
the param to `roles/writer`, `roles/resourcemanager.projectIamAdmin`,
`roles/iam.serviceAccountAdmin`, `roles/resourcemanager.projectDeleter`,
`roles/billing.projectManager` and the rest of the list; its plan then shows two destroys and four
creates. The default changes to that set in a later minor, with a `## Breaking changes` entry, when
Google's roles overview no longer marks Admin, Writer and Reader as Preview.

The standing read side stays `roles/viewer`, which is GA and granted throughout the library;
`roles/reader` is a separate decision for the same moment.

## Consequences

- `gcp_project_admins_roles` is the first per-group role list param; the ones for the other groups
  follow its spelling.
- An estate that binds Writer loses, for the project admins, custom-role creation at the
  organisation, VPC Service Controls perimeter edits and PAM entitlement creation, and the legacy
  convenience grants; an estate whose platform team uses one of those adds the named role for it.
- Before an estate binds the set, the group's 90-day usage of Owner is read (the IAM policy
  insight) and every exercised permission Writer lacks becomes a named role in the list.
- The model keeps `roles/iam.denyAdmin` off every group below the organisation admins, because a
  deny policy is the one thing below Owner's level that cuts an inherited grant.
- Once an estate holds Writer, the five roles are the project admins' elevated tier in the
  privileged-access add-on, and the rest of the list its standing tier.
