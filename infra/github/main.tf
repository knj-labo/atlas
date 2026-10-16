provider "github" {
  owner = "knj-labo"
}

resource "github_repository" "atlas" {
  name       = "atlas"
  visibility = "public"

  has_issues      = true
  has_projects    = true
  has_wiki        = true
  has_discussions = false

  allow_squash_merge     = true
  allow_merge_commit     = false
  allow_rebase_merge     = false
  allow_auto_merge       = false
  allow_update_branch    = false
  delete_branch_on_merge = false

  squash_merge_commit_title   = "COMMIT_OR_PR_TITLE"
  squash_merge_commit_message = "COMMIT_MESSAGES"

  lifecycle {
    prevent_destroy = true
  }
}
