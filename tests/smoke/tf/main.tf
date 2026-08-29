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

resource "google_storage_bucket" "logs" {
  name     = "corp-logs-001"
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

resource "google_project" "infra" {
  name       = "corp-IaC"
  project_id = "corp-infra-001"
  folder_id  = google_folder.workloads.name
}

resource "google_project_service" "infra_iam" {
  project = google_project.infra.project_id
  service = "iam.googleapis.com"
}

resource "google_project_iam_member" "infra_viewer" {
  project = google_project.infra.project_id
  role    = "roles/viewer"
  member  = "group:gcp-auditors@example.com"
}

resource "google_organization_iam_member" "admins" {
  org_id = "123456789012"
  role   = "roles/resourcemanager.organizationViewer"
  member = "group:gcp-organization-admins@example.com"
}
