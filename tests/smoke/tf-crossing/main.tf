# A Terraform fixture whose reference crosses the boundary the hcl import draws:
# the bucket cannot be translated (`for_each`), the grant on it can, and the
# grant names the bucket. satz emits no address for a verbatim block, so the
# estate would not transpile — the import refuses and writes nothing.
# Example values only.
variable "bucket_names" {
  type    = list(string)
  default = ["corp-state-001"]
}

resource "google_project" "infra" {
  name       = "corp-IaC"
  project_id = "corp-infra-001"
  org_id     = "123456789012"
}

resource "google_storage_bucket" "state" {
  for_each = toset(var.bucket_names)
  name     = each.value
  location = "EU"
  project  = google_project.infra.project_id
}

resource "google_storage_bucket_iam_member" "state_reader" {
  bucket = google_storage_bucket.state.name
  role   = "roles/storage.objectViewer"
  member = "group:gcp-auditors@example.com"
}
