# winget

`AkhileshVarute.Nock.yaml` is the manifest for the Windows Package Manager, so people can run
`winget install AkhileshVarute.Nock`.

To publish a new version, after the GitHub release is built:

```
wingetcreate update AkhileshVarute.Nock --version <x.y.z> --urls https://github.com/Akhilesh-Varute/Nock/releases/download/v<x.y.z>/NockSetup-<x.y.z>.exe --submit
```

The first submission is a pull request to `microsoft/winget-pkgs` (copy the file to
`manifests/a/AkhileshVarute/Nock/<version>/` as three files, or let `wingetcreate new` do it).
Test locally first: `winget validate winget\AkhileshVarute.Nock.yaml`.
