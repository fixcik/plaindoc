# Building

```sh
cargo build --release -p plaindoc-cli    # target/release/plaindoc
```

The default CLI build (feature `ocr`) embeds the two pinned OCR models, PP-OCRv5 mobile det and
cyrillic rec. Both are Apache-2.0 and together about 13 MB. Their URLs and sha256 are listed in
`crates/ocr/model-manifest.txt`. The model files are not in git.

## Where the build gets the models

`crates/ocr/build.rs` takes each model from the first place that has it:

1. `$PLAINDOC_OCR_BUNDLE_DIR/<id>/inference.onnx`
2. `crates/ocr/models/<id>/inference.onnx`. This directory is git-ignored and is filled by
   `scripts/fetch-ocr-models.sh`.
3. The user models dir, which the binary also reads at run time:
   - macOS: `~/Library/Application Support/plaindoc/models`
   - Linux: `$XDG_DATA_HOME/plaindoc/models` or `~/.local/share/plaindoc/models`
   - Windows: `%LOCALAPPDATA%\plaindoc\models`

If a model is in none of these, the build downloads it into the user models dir. The download
uses `curl`, checks the sha256 and writes the file atomically. This happens once; later builds
reuse the cached files and do not touch the network.

A file in place 1 or 2 with the wrong sha256 is a build error. A wrong file in the user models
dir is downloaded again.

## Three ways to get models

1. **Plain build.** Run `cargo build`. The models are downloaded on first build and embedded.
2. **Offline build.** Run `scripts/fetch-ocr-models.sh` once on a machine with network access. It
   fills `crates/ocr/models`; you can also pass a directory and point `PLAINDOC_OCR_BUNDLE_DIR`
   at it. Then build with `PLAINDOC_NO_FETCH=1`.
3. **Model-less binary.** Build with `PLAINDOC_NO_FETCH=1` and no models present. Note that
   `--no-default-features` is a different build: it has no OCR engine at all. Supply the models to
   a model-less binary at run time:
   ```sh
   scripts/fetch-ocr-models.sh DIR
   plaindoc --ocr-models DIR parse scan.pdf   # or PLAINDOC_OCR_MODELS=DIR
   ```
   The user models dir is also searched at run time.

   `PLAINDOC_NO_FETCH=1` alone does not give a model-less build on a machine that has built
   once: the user models dir is searched first and already holds the models. Use a fresh `HOME`
   (or remove that dir) for a really model-less binary.

`plaindoc models list` shows, for each model, whether it is bundled or where it was looked
for.

## Build environment variables

| Variable | Effect |
|---|---|
| `PLAINDOC_OCR_BUNDLE_DIR=DIR` | Look in `DIR` first. |
| `PLAINDOC_NO_FETCH=1` | Never download. If the models are not found, build without them and print a warning. |
| `CARGO_NET_OFFLINE=true` | Same as `PLAINDOC_NO_FETCH=1`. Only the variable is detected: `cargo --offline` is not passed to build scripts. |
| `PLAINDOC_REQUIRE_MODELS=1` | Missing models or a failed download are a build error. Use it for release builds and CI. `PLAINDOC_OCR_REQUIRE_BUNDLE=1` is an older alias. |
| `PLAINDOC_CURL=PATH` | Use this `curl` binary. |

Without `PLAINDOC_REQUIRE_MODELS`, a failed download (offline, no `curl`, proxy) does not fail
the build. It prints `warning: plaindoc-ocr@0.1.0: feature 'bundled-models': cannot get OCR
model …; building without embedded OCR models`. The build script reruns when a file appears in one of the model directories, so it picks the
models up then.

## A binary without models

The binary still parses everything. Pages that need OCR, such as scans, come out empty. The
document then carries an `OcrUnavailable` warning at level `warn`, which is printed to stderr and
embedded in Markdown. The warning says that the OCR models are missing and how to provide them.
The exit code is unchanged, but with `--strict --min-quality` it can change: a docx/pptx with
pictures on a model-less binary drops from quality 1.0 to 0.9.

## Releases

1. Set `version` in `[workspace.package]` of `Cargo.toml` and commit.
2. Tag the commit `vX.Y.Z` (the same version) and push the tag: `git push origin vX.Y.Z`.

`.github/workflows/release.yml` checks that the tag matches the version, builds `plaindoc` with
`PLAINDOC_REQUIRE_MODELS=1` for each target, checks that the binary has no non-system dynamic
dependencies, and runs a smoke test with an empty user models dir. It then packs the binary with
`scripts/package-release.sh` and creates a GitHub Release with the archives, `SHA256SUMS` and
notes generated from the commits. Running the workflow by hand (`workflow_dispatch`) does the
same steps but publishes nothing; the archives are kept as workflow artifacts.

There is no `x86_64-apple-darwin` build: the ONNX Runtime prebuilts used by `ort` do not cover it.
