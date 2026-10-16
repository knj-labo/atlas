# GitHub repository settings

`infra/github` manages the existing `knj-labo/atlas` repository with Terraform.
Pull requests can only use squash merging. The other declared settings retain
their values from before Terraform adoption.

Use Terraform 1.5 or newer (before 2.0) and authenticate with `gh auth login` as
an account that can administer this repository. The GitHub provider uses the
GitHub CLI credentials when `GITHUB_TOKEN` is not set. An administrative token
can also be supplied through that environment variable; do not commit tokens.
Leave `GITHUB_OWNER` and `GITHUB_ORGANIZATION` unset so the provider uses the
`knj-labo` owner declared in the configuration.

From the repository root:

```sh
terraform -chdir=infra/github init -lockfile=readonly
terraform -chdir=infra/github fmt -check
terraform -chdir=infra/github validate
terraform -chdir=infra/github plan -out=settings.tfplan
terraform -chdir=infra/github apply settings.tfplan
terraform -chdir=infra/github plan -detailed-exitcode
```

Inspect the saved plan before applying it. The initial plan imports `atlas`
and disables merge commits and rebase merging. It must not create or destroy a
repository. Subsequent plans should report no changes unless configuration or
GitHub settings have changed; the final command exits with 0 for no changes,
2 for a proposed change, or 1 for an error.

State and saved plans stay local and are ignored by Git. Keep the local state
between runs and run applies from one checkout at a time. A fresh checkout uses
the import block to adopt the existing repository again. Commit the provider
lock file, `.terraform.lock.hcl`, so each checkout uses the same provider.
The resource also declares `prevent_destroy` to reject repository deletion
while it remains in the configuration.

See the provider's [repository resource documentation](https://registry.terraform.io/providers/integrations/github/6.13.0/docs/resources/repository)
and [authentication documentation](https://registry.terraform.io/providers/integrations/github/6.13.0/docs).
