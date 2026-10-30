# winget

`manifests/a/AkhileshVarute/Nock/<version>/` holds the three manifest files for the Windows Package Manager, so
people can run `winget install AkhileshVarute.Nock`. The first version was submitted to `microsoft/winget-pkgs`;
every release after that is an update PR.

## Release checklist (do all of it, every time)

1. Bump `version` in `Cargo.toml`, commit, push a branch, open a PR on `Akhilesh-Varute/Nock` and merge it.
2. Tag `v<x.y.z>` and push the tag. The Release workflow builds `NockSetup-<x.y.z>.exe` and `Nock-portable-<x.y.z>.exe`.
3. Update winget with the NEW version and the NEW installer hash:

   ```
   wingetcreate update AkhileshVarute.Nock --version <x.y.z> --urls https://github.com/Akhilesh-Varute/Nock/releases/download/v<x.y.z>/NockSetup-<x.y.z>.exe --submit
   ```

   (or copy the folder to the new version, change `PackageVersion`, `InstallerUrl` and `InstallerSha256`, run
   `winget validate`, and open a PR to `microsoft/winget-pkgs`).
4. Keep README, the site demo and these manifests in step with the released version.
