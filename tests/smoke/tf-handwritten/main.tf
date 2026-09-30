terraform {
  required_providers {
    google = {
      source = "hashicorp/google"
    }
  }
}

# the project the provider defaults to is built from the organisation
locals {
  infra_project_id = "${var.project_prefix}${var.org_id}"
}

provider "google" {
  project = local.infra_project_id
  region  = "europe-west3"
}

resource "google_project" "infra" {
  name            = "Infra"
  project_id      = local.infra_project_id
  org_id          = var.org_id
  billing_account = var.billing_account
}

resource "google_project_service" "apis" {
  count   = length(var.enable_apis)
  project = google_project.infra.project_id
  service = element(var.enable_apis, count.index)
}

# names no project: the provider's default is where it lives
resource "google_service_account" "onboarding" {
  account_id   = "onboarding"
  display_name = "Onboarding"
}

resource "google_organization_iam_custom_role" "reader" {
  org_id      = var.org_id
  role_id     = "acmeReader"
  title       = "Acme reader"
  permissions = ["resourcemanager.projects.get", "resourcemanager.folders.get"]
}

# authoritative: carried verbatim, and it still reads the organisation variable
resource "google_organization_iam_binding" "viewer" {
  org_id  = var.org_id
  role    = "roles/viewer"
  members = ["serviceAccount:${google_service_account.onboarding.email}"]
}
