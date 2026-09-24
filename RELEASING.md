# Releasing

The release process consists of following steps:

- Run the release script from a clean, up-to-date `main` branch. The `version` should not contain a leading `v`.

  ```console
  scripts/release.sh <version>
  ```
- Confirm pushing the release commit. The script will automatically create a release pull request.
- Review and merge the pull request.
- Wait for CI on the merged commit to pass and confirm that the release commit is still the tip of `main`.
- Come back to the release script (it should be paused) and confirm creating and pushing the release tag. The script will create a draft GitHub release and a commit-specific Docker snapshot.
- Before publishing the draft, verify that the workflows passed and the release notes and prerelease status are correct.
- Publishing the draft triggers crates.io publication and the final Docker tags. This also applies to prereleases, and published crates cannot be overwritten.

After publishing the release:

- Confirm that the new version is available on crates.io for `pathfinder-crypto`, `pathfinder-common`, `pathfinder-serde`, `pathfinder-class-hash`, and `pathfinder-consensus`.
- Confirm that `v<version>` is available on [Docker Hub](https://hub.docker.com/r/swmansion/pathfinder). For a stable release, confirm that `latest` has the same digest.
- For a stable release, confirm that the infrastructure version-update pull request was created.
