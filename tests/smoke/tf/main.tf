# A hand-written Terraform fixture for the hcl import shape (example values only).
terraform {
  required_providers {
    google = {
      source = "hashicorp/google"
    }
  }
}

provider "google" {
  project = "corp-infra-001"
  region  = "europe-west3"
}

resource "google_folder" "workloads" {
  display_name = "Workloads"
  parent       = "organizations/123456789012"
}

variable "bucket_suffix" {
  type    = string
  default = "001"
}

resource "google_storage_bucket" "logs" {
  # an alias that is not the estate's own: a Satz body key, so it travels with
  # the resource and the emitter renders it back as the reference it was
  provider = google-beta.google-beta

  # a template over a promoted variable: resolves to the same literal it did
  name     = "corp-logs-${var.bucket_suffix}"
  location = "EU"
  project  = "corp-infra-001"

  uniform_bucket_level_access = true

  lifecycle_rule {
    action {
      type = "Delete"
    }
    condition {
      age = 30
    }
  }
}

locals {
  env = "prod"
}

# a bucket-scoped grant: NOT a member map (the map form has no room for
# `bucket`), so it translates as a labelled resource, and its bucket reference
# is carried verbatim
resource "google_storage_bucket_iam_member" "logs_reader" {
  bucket = google_storage_bucket.logs.name
  role   = "roles/storage.objectViewer"
  member = "group:gcp-auditors@example.com"
}

resource "google_project" "infra" {
  name       = "corp-IaC"
  project_id = "corp-infra-001"
  folder_id  = google_folder.workloads.name
}

# the estate's own provider, which the emitter writes on every resource that
# names no other, and an ordering edge satz derives from the estate itself:
# both are dropped, and the service still becomes an entry in the project's list
resource "google_project_service" "infra_iam" {
  provider   = google.google
  project    = google_project.infra.project_id
  service    = "iam.googleapis.com"
  depends_on = [google_project.infra]
}

resource "google_project_iam_member" "infra_viewer" {
  project    = google_project.infra.project_id
  role       = "roles/viewer"
  member     = "group:gcp-auditors@example.com"
  depends_on = [google_project_service.infra_iam]
}

resource "google_organization_iam_member" "admins" {
  org_id = "123456789012"
  role   = "roles/resourcemanager.organizationViewer"
  member = "group:gcp-organization-admins@example.com"
}

# Terraform's "one of these per entry": in Satz that IS one resource per entry,
# so the import expands it rather than carrying it verbatim.
variable "log_viewers" {
  type    = list(string)
  default = ["group:gcp-auditors@example.com", "group:gcp-organization-admins@example.com"]
}

resource "google_project_iam_member" "log_viewers" {
  count   = length(var.log_viewers)
  project = google_project.infra.project_id
  role    = "roles/logging.viewer"
  member  = var.log_viewers[count.index]
}
