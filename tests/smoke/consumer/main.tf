# A project's HCL beside the showcase estate, for `satz check-consumer`:
# one attachment at an attach point, and one onto an export that is none.

module "satz" {
  source = "../estate/interfaces/archive/archive/hcl"
}

# allowed: `archive_project_id` is an attach point for google_project_iam_member
resource "google_project_iam_member" "archive_readers" {
  project = module.satz.archive_project_id
  role    = "roles/viewer"
  member  = "group:gcp-auditors@example.com"
}

# refused: `infra_folder` is read, and takes no attachment
resource "google_folder_iam_member" "infra_readers" {
  folder = module.satz.infra_folder
  role   = "roles/viewer"
  member = "group:gcp-auditors@example.com"
}
