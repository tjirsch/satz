# A hand-written onboarding configuration in the shape vendors generate for a
# customer to apply (example values only). The organisation is asked for, never
# defaulted.
variable "org_id" {
  type        = string
  description = "The organization id"
}

variable "billing_account" {
  type    = string
  default = "012345-6789AB-CDEF01"
}

variable "project_prefix" {
  type    = string
  default = "acme-infra-"
}

# one entry twice: Terraform creates two resources that manage the same service
variable "enable_apis" {
  type    = list(string)
  default = ["iam.googleapis.com", "logging.googleapis.com", "sts.googleapis.com", "logging.googleapis.com"]
}
