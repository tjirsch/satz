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
