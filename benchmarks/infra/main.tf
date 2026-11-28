# GCP resources for the benchmark workflow (.github/workflows/benchmark.yml).
#
# To set up a new GCP project:
#
#   cd benchmarks/infra
#   terraform init
#   terraform apply -var project_id=<project> -var github_repository=<owner>/<repo>
#
# Then add each value from `terraform output` as a GitHub variable of the
# repository, under Settings, Secrets and variables, Actions, Variables.
#
# Terraform keeps its state in a local file. You need that file to change or
# remove the resources later, so keep it or add a GCS backend.

terraform {
  required_version = ">= 1.5"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 6.0"
    }
  }
}

variable "project_id" {
  description = "GCP project to create the resources in."
  type        = string
}

variable "region" {
  description = "Region of the bucket and the VM subnet."
  type        = string
  default     = "europe-west1"
}

variable "zone" {
  description = "Zone of the benchmark VMs. Must be in the region."
  type        = string
  default     = "europe-west1-b"
}

variable "machine_type" {
  description = "Machine type of the benchmark VMs. Results are comparable only on the same type."
  type        = string
  default     = "n2-standard-8"
}

variable "github_repository" {
  description = "Repository allowed to run benchmarks, as owner/name."
  type        = string
}

variable "existing_github_provider" {
  description = <<-EOT
    An existing workload identity provider for GitHub Actions to use, as
    POOL_ID/PROVIDER_ID. It must map `attribute.repository`. Leave empty to
    create a pool that trusts only `github_repository`. Creating one needs the
    Workload Identity Pool Admin role.
  EOT
  type        = string
  default     = ""
}

variable "results_retention_days" {
  description = "Days to keep run results in the bucket."
  type        = number
  default     = 180
}

variable "binaries_retention_days" {
  description = "Days to keep cached binaries and compiled crates (sccache) in the bucket."
  type        = number
  default     = 30
}

provider "google" {
  project = var.project_id
  region  = var.region
}

resource "google_project_service" "apis" {
  for_each = toset([
    "compute.googleapis.com",
    "iam.googleapis.com",
    "iamcredentials.googleapis.com",
    "sts.googleapis.com",
    "storage.googleapis.com",
  ])
  service            = each.value
  disable_on_destroy = false
}

# Cached binaries (bin/) and run results (runs/).
resource "google_storage_bucket" "bench" {
  name                        = "${var.project_id}-pathfinder-bench"
  location                    = var.region
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  lifecycle_rule {
    condition {
      age            = var.binaries_retention_days
      matches_prefix = ["bin/", "sccache/"]
    }
    action {
      type = "Delete"
    }
  }

  lifecycle_rule {
    condition {
      age            = var.results_retention_days
      matches_prefix = ["runs/"]
    }
    action {
      type = "Delete"
    }
  }

  depends_on = [google_project_service.apis]
}

# Its own network, so the setup works in projects without a default network.
# It has no ingress rules, so nothing can connect to the VMs.
resource "google_compute_network" "bench" {
  name                    = "pathfinder-bench"
  auto_create_subnetworks = false
  depends_on              = [google_project_service.apis]
}

resource "google_compute_subnetwork" "bench" {
  name          = "pathfinder-bench"
  network       = google_compute_network.bench.id
  region        = var.region
  ip_cidr_range = "10.10.0.0/24"
}

# What every benchmark VM looks like. The workflow creates each VM from this
# template and adds the startup script and the job as metadata.
#
# One thread per core, so the two hyperthreads of a core don't disturb each
# other's timings. GCE deletes the VM after 6 hours, the GitHub job limit, in
# case the workflow never does. The external IP is only for outgoing traffic:
# the network has no ingress rules.
resource "google_compute_instance_template" "bench" {
  name         = "pathfinder-bench"
  machine_type = var.machine_type

  # Only the system, the binaries and the logs. The database will go on local
  # SSDs, not on this disk.
  disk {
    source_image = "debian-cloud/debian-12"
    disk_size_gb = 20
    boot         = true
    auto_delete  = true
  }

  network_interface {
    subnetwork = google_compute_subnetwork.bench.id
    access_config {}
  }

  service_account {
    email  = google_service_account.vm.email
    scopes = ["cloud-platform"]
  }

  advanced_machine_features {
    threads_per_core = 1
  }

  scheduling {
    instance_termination_action = "DELETE"
    max_run_duration {
      seconds = 6 * 60 * 60
    }
  }
}

# The benchmark VMs run as this account. It reads binaries and writes results.
resource "google_service_account" "vm" {
  account_id   = "pathfinder-bench-vm"
  display_name = "Pathfinder benchmark VM"
  depends_on   = [google_project_service.apis]
}

resource "google_storage_bucket_iam_member" "vm_bucket_access" {
  bucket = google_storage_bucket.bench.name
  role   = "roles/storage.objectAdmin"
  member = google_service_account.vm.member
}

# The workflow runs as this account. It uploads binaries and creates and
# deletes VMs.
resource "google_service_account" "github" {
  account_id   = "pathfinder-bench-github"
  display_name = "Pathfinder benchmark workflow"
  depends_on   = [google_project_service.apis]
}

resource "google_storage_bucket_iam_member" "github_bucket_access" {
  bucket = google_storage_bucket.bench.name
  role   = "roles/storage.objectAdmin"
  member = google_service_account.github.member
}

resource "google_project_iam_member" "github_manages_vms" {
  project = var.project_id
  role    = "roles/compute.instanceAdmin.v1"
  member  = google_service_account.github.member
}

# Lets the workflow start VMs that run as the VM service account.
resource "google_service_account_iam_member" "github_acts_as_vm" {
  service_account_id = google_service_account.vm.name
  role               = "roles/iam.serviceAccountUser"
  member             = google_service_account.github.member
}

# GitHub Actions logs in with its OIDC token, so no key is stored anywhere.
# The pool and provider below are created only without
# `existing_github_provider`. Either way, only workflows of
# `github_repository` can act as the workflow account.
resource "google_iam_workload_identity_pool" "github" {
  count                     = var.existing_github_provider == "" ? 1 : 0
  workload_identity_pool_id = "pathfinder-bench-github"
  display_name              = "Pathfinder benchmark GitHub"
  depends_on                = [google_project_service.apis]
}

resource "google_iam_workload_identity_pool_provider" "github" {
  count                              = var.existing_github_provider == "" ? 1 : 0
  workload_identity_pool_id          = google_iam_workload_identity_pool.github[0].workload_identity_pool_id
  workload_identity_pool_provider_id = "github"
  display_name                       = "GitHub Actions"
  attribute_mapping = {
    "google.subject"       = "assertion.sub"
    "attribute.repository" = "assertion.repository"
  }
  attribute_condition = "assertion.repository == '${var.github_repository}'"
  oidc {
    issuer_uri = "https://token.actions.githubusercontent.com"
  }
}

data "google_project" "this" {}

locals {
  github_pool = (
    var.existing_github_provider == ""
    ? google_iam_workload_identity_pool.github[0].name
    : "projects/${data.google_project.this.number}/locations/global/workloadIdentityPools/${split("/", var.existing_github_provider)[0]}"
  )
  github_provider = (
    var.existing_github_provider == ""
    ? google_iam_workload_identity_pool_provider.github[0].name
    : "${local.github_pool}/providers/${split("/", var.existing_github_provider)[1]}"
  )
}

resource "google_service_account_iam_member" "github_login" {
  service_account_id = google_service_account.github.name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${local.github_pool}/attribute.repository/${var.github_repository}"
}

# The GitHub variables the workflow reads.
output "BENCH_GCP_PROJECT" {
  value = var.project_id
}

output "BENCH_GCP_ZONE" {
  value = var.zone
}

output "BENCH_INSTANCE_TEMPLATE" {
  value = google_compute_instance_template.bench.name
}

output "BENCH_BUCKET" {
  value = google_storage_bucket.bench.name
}

output "BENCH_WIF_PROVIDER" {
  value = local.github_provider
}

output "BENCH_GITHUB_SERVICE_ACCOUNT" {
  value = google_service_account.github.email
}
