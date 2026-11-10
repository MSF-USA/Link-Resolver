# Releasing Link Resolver

Releases are manual. Nothing is published when commits land on `main`; a maintainer
starts every release on purpose. This page is the whole procedure.

## Version scheme

Versions are `v1.DATE`, where DATE is the UTC day the release was started, written as
`YYYYMMDD`. The first release on 6 November 2026 is `v1.20261106`. A second release the
same day is `v1.20261106.2`, a third `v1.20261106.3`, and so on. The workflow picks the
next free number itself.

The binaries report the same version with Cargo's three-part form: `v1.20261106` builds
as crate version `1.20261106.0`, and `v1.20261106.2` as `1.20261106.2`. That is the
version in the HTTP user agent (`LinkResolver/1.20261106.0`) and in `--help` output, so a
support request can be matched to a release. The version is stamped into `Cargo.toml` at
build time by `packaging/stamp-version.py`, which edits the manifest and the crate's own
entry in `Cargo.lock` directly (no cargo command, no network), and is never committed;
the repository keeps `0.1.0`.

## Before you start

- CI is green on the commit you want to release (Actions tab, "CI" workflow). The release
  workflow runs the full test suite again on all three platforms, but a known-red commit wastes
  a run.
- For a change to the packaging scripts or the workflow itself, run the local rehearsal
  first. It copies the tracked files to a scratch directory, stamps a version, runs every
  test suite and builds the Linux archives exactly as the Linux release job does, then
  checks the packaged binaries report that version:

  ```sh
  packaging/rehearse-release.sh            # today's date
  packaging/rehearse-release.sh v1.20261106.2
  ```

  It needs the tools listed at the top of `packaging/package-dist.sh` except MinGW. The
  Windows half (`packaging/package-windows.ps1`) can only be rehearsed on a Windows
  machine: run `cargo test --locked`, then the script, then `--version` on the packaged
  executable.
  On an Apple Silicon Mac, run `cargo test --locked`, then
  `python3 packaging/package-macos.py`. This checks Mach-O architecture, macOS
  deployment versions, system-only dependencies, CLI version, bundle metadata and
  ad-hoc signatures. Native GUI interaction is not exercised by the headless tests.
- `README.md` and `QUICKSTART.txt` describe the behaviour being shipped.
- You have write access to the repository. The workflow uses the repository's own token;
  no secrets need to be configured.

The tests use loopback HTTP servers only. No job in the release contacts a real URL, so a
release can be cut from any network. Real link resolution only ever happens on the
computers of the people who run the app.

## Option A: from the Actions tab (recommended)

1. Open the repository on GitHub, choose **Actions**, then the **Release** workflow.
2. Click **Run workflow**.
3. Leave the branch on `main` unless you are releasing a hotfix branch.
4. Optionally type release notes. They appear above the automatically generated list of
   merged pull requests. Tick **pre-release** for a build you want people to try before it
   becomes the default download.
5. Click the green **Run workflow** button.

The workflow then:

1. Computes the version from today's UTC date and the existing tags and releases.
2. Builds and tests on Linux, Windows and Apple Silicon macOS in parallel, with the version stamped in.
   Linux archives come from `packaging/package-dist.sh --linux-only` (glibc 2.28 baseline
   and a static musl CLI). Windows archives are built natively with the MSVC toolchain by
   `packaging/package-windows.ps1`, which also checks the executables' subsystems and runs
   `link-resolver-cli.exe --help`.
   macOS archives come from `packaging/package-macos.py` on `macos-15` (ARM64),
   with a macOS 13.0 deployment target and a Finder-launchable `.app` bundle.
   They are ad-hoc signed, not Developer ID signed or notarized; no Apple credentials
   are needed. Gatekeeper guidance is included in the archives and release notes.
3. Only once all three builds pass, tags the chosen commit with the new version and pushes the
   tag.
4. Publishes the GitHub release with the six archives, `SHA256SUMS`, your notes, a
   downloads table and the generated change list.

The run takes roughly ten to fifteen minutes. The release appears under **Releases** on
the repository page.

## Option B: push a tag

Use this when the version must be chosen by hand, for example to re-run a release that
failed after its tag was pushed.

```sh
git switch main && git pull
git tag -a v1.20261106 -m "Release v1.20261106"
git push origin v1.20261106
```

The tag must match `v1.YYYYMMDD` or `v1.YYYYMMDD.N`; anything else fails in the first
step with a message saying so. The workflow then runs steps 2 and 4 above; it does not
create another tag.

## After the release

1. Open the release page and download `SHA256SUMS` and one archive. Check the hash:
   `sha256sum -c SHA256SUMS --ignore-missing` on Linux, or
   `Get-FileHash link-resolver-gui-windows-x64.zip` on Windows.
2. Unpack the archive for your platform. `link-resolver-cli --version` prints
   `Link Resolver 1.20261106.0` for `v1.20261106`, and the desktop app shows the same
   version at the bottom of its Settings dialog. The packagers already checked this on
   all three runners, so this is a last look rather than a test.
3. Share the release page link. The archives need no installation; `QUICKSTART.txt` inside
   each one explains the rest.

## If something fails

- **A build or test job fails.** Nothing has been tagged or published. Fix the problem on
  `main` and start again; the same date version is still free.
- **The publish job fails after tagging.** The tag exists but there is no release. Either
  delete the tag (`git push origin :refs/tags/v1.20261106`) and run Option A again, or
  fix the cause and push the same tag again with Option B.
- **A release must be withdrawn.** Delete it from the Releases page and delete its tag.
  The next release that day takes the next free suffix.
- **Dependabot or CI needs a newer Zig or cargo-zigbuild.** Both versions are pinned at
  the top of `.github/workflows/release.yml` and must be bumped together, after checking
  that `packaging/package-dist.sh` still passes locally with the same pair.

## Building the archives by hand

A maintainer with the tools from the top of `packaging/package-dist.sh` (MinGW, Zig,
cargo-zigbuild, the two extra Rust targets) can build all four archives locally into
`dist/`:

```sh
packaging/stamp-version.py v1.20261106   # optional, to match a release version
packaging/package-dist.sh
git checkout Cargo.toml Cargo.lock       # drop the stamped version afterwards
```

On Windows, `powershell -ExecutionPolicy Bypass -File packaging\package-windows.ps1`
builds the two Windows archives natively. Local archives are for testing; published
releases should always come from the workflow so that the tag, the checksums and the
binaries are produced together.

On macOS, `python3 packaging/package-macos.py` builds
`link-resolver-gui-macos-arm64.zip` and `link-resolver-macos-arm64.zip`, plus
`SHA256SUMS-macos-arm64`. Requires Rust 1.95+, Python 3.11+ and Xcode Command Line
Tools. `--target x86_64-apple-darwin` produces Intel archives on a compatible host
with that Rust target installed; Intel archives are not currently published.
`--deployment-target 13.0` selects the baseline (the default). The binary load
commands and app bundle must agree with it; test older baselines on real hardware
before advertising support. macOS archives preserve executable permissions.

The app's displayed version is the stamped crate version. Apple's internal
`CFBundleVersion` has stricter 4/2/2-digit limits: date-based releases encode the
date ordinal into two base-100 fields, followed by the release suffix (0..99).
This preserves chronological order without putting eight digits in Apple's
two-digit field. Development version `0.1.0` uses build version `1.1.0`.

## macOS signing limitations

Ad-hoc signatures satisfy Apple Silicon's executable-signature requirement but
do not establish developer identity or remove Gatekeeper warnings. Organization
policies may block these downloads entirely. Users should only use Open Anyway
for trusted downloads; no instructions disable Gatekeeper globally.

A future notarized release requires an Apple Developer membership, Developer ID
Application certificate, secure temporary CI keychain, hardened-runtime signing
of both the app and CLI, submission through `notarytool`, and stapling the app's
ticket before archiving. Those credentials and steps are deliberately not part
of the unsigned initial distribution. OpenGL remains the GUI renderer; switching
to Metal is independent of adding macOS builds.
