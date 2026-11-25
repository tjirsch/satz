# 0107 — an `entries` default may read another field of the same entry

- **Status:** accepted
- **Date:** 2026-11-25

## Context

`presets/delegated-projects.satz` gave each project the entry's `name` as its display name.
Google takes a project display name of 4 to 30 characters, so an entry named `rag`
transpiled and failed at `tofu validate`. `presets/delegated-folders.satz` already had an
optional `display_name`, written as two `each … when display_name` / `when not
display_name` bodies, and no check of the value at all; a folder display name has a rule of
its own (at most 30 characters, starting and ending with a letter or digit).

A list's `patterns` are the place satz holds an entry's field to a form: they are checked on
the filled entry at every transpile, by `satz add-entry` and by `satz check-request`, and the
refusal names the entry, the field, the value and the pattern. A pattern only sees a field the
entry carries, so a display name that is the `name` when left out is checked only when the
default fills it in. A default was a value or a param, resolved once for the whole list.

## Options

**A. A default reads another field of the entry, `display_name = each.name`.** The field is
filled per entry, before the patterns and every reader of the list see it, so one pattern on
`display_name` covers both an entry that states one and one that does not. Costs a second
form of default in the language and in the resolved `entries` statement (`field_defaults`,
also in `satz interfaces --format json`). The field it reads must carry no default of its
own, so the order of the defaults never matters.

**B. Two `when` bodies, a pattern on `display_name` alone.** No language change; what the
folder pack did. An entry without a display name is not checked — the case that failed.

**C. Tighten the `name` pattern to the display name's rule.** No language change. Refuses
`rag` even with a `display_name` given, and a folder's rule (ending in a letter or digit)
would forbid names the label and the interface take today.

**D. Make `display_name` required.** No language change; every existing entry needs an
edit for a value that is its name in almost every estate.

## Decision

A. `defaults = { display_name = each.name }`. Refused: `each.<field>` of a field `fields`
does not name, of the field itself, or of a field that takes a default. An entry that does
not carry the field read is left without the defaulted field. `delegated_projects` 2.1 and
`delegated_folders` 1.2 use it with a pattern of Google's rule for the display name.

## Consequences

- A display name Google refuses is refused by satz, naming the entry, before a plan.
- `delegated_folders` writes one `google_folder` body; `terraform.tfvars` carries each
  entry's filled `display_name`.
- `satz interfaces --format json` reports such a default under `field_defaults`, the field
  per field, beside `defaults`.
