# Installing markdoc-pdf

`markdoc-pdf` is a single binary. It has no runtime dependencies
beyond the OS — no Java, no Python, no Pandoc — and produces PDFs
out of the box (the bundled Noto Sans family covers Latin scripts;
custom fonts are loadable via the style file).

## Option A: install from source with Cargo (Linux / macOS)

The easiest path if you already have Rust installed.

**Prerequisites**: Rust 1.93 or newer. If you don't have Rust:

```sh
# Linux / macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

`markdoc-pdf` depends on the sibling crates `markdoc` and
`flux-types` via relative `path = "../markdoc"` references. Clone
all three side-by-side first:

```sh
mkdir entsp && cd entsp
git clone https://github.com/entsp/markdoc.git
git clone https://github.com/entsp/flux-types.git
git clone https://github.com/entsp/markdoc-pdf.git
cd markdoc-pdf
cargo install --path . --locked
```

This builds the release binary and copies it into `~/.cargo/bin/`.
Make sure that's on your `$PATH`:

```sh
echo $PATH | tr ':' '\n' | grep cargo
# Should print something like: /home/you/.cargo/bin
```

Verify:

```sh
markdoc-pdf --version
```

## Option B: build the binary and copy it manually (Linux / macOS)

Useful when you want to drop the binary into `/usr/local/bin/` or
hand it to someone without giving them the source tree.

```sh
cd markdoc-pdf
cargo build --release --bin markdoc-pdf
sudo cp target/release/markdoc-pdf /usr/local/bin/
markdoc-pdf --version
```

The release binary is a single ~25 MB executable. Strip it for
distribution:

```sh
strip target/release/markdoc-pdf       # ~12-15 MB after strip
```

## Option C: Windows

Two paths — pick whichever fits your shell.

### C1. PowerShell + rustup

1. Install **Rust** via the official installer:
   <https://www.rust-lang.org/tools/install>. Pick the **MSVC**
   toolchain when prompted (default). The installer also installs
   the Visual Studio Build Tools if you don't already have them.
2. Clone the three sibling repos into one parent folder:

   ```powershell
   mkdir entsp; cd entsp
   git clone https://github.com/entsp/markdoc.git
   git clone https://github.com/entsp/flux-types.git
   git clone https://github.com/entsp/markdoc-pdf.git
   cd markdoc-pdf
   cargo install --path . --locked
   ```
3. The binary lands at `%USERPROFILE%\.cargo\bin\markdoc-pdf.exe` —
   already on PATH thanks to the rustup installer. Verify:

   ```powershell
   markdoc-pdf --version
   ```

### C2. WSL (Windows Subsystem for Linux)

If you're already comfortable with WSL2 (Ubuntu, Debian, etc.), the
**Linux/macOS instructions in Option A apply unchanged**. The
binary lives inside the WSL filesystem and is only visible from
WSL — fine if you also edit your `.mdoc` files in WSL or via VS Code's
Remote-WSL.

To use a WSL-built binary from Windows-side tooling, copy it across:

```sh
# from inside WSL
cp ~/.cargo/bin/markdoc-pdf /mnt/c/Users/$USER/bin/
```

Then add `C:\Users\<you>\bin\` to your Windows PATH.

### Windows-specific notes

- Use `\` or `/` in path arguments — both work.
- The `--input` and `--output` flags accept absolute Windows paths
  (`C:\Users\you\Docs\manual.mdoc`); quote them if they contain spaces.
- File paths in your `.mdoc` (e.g. `![alt](images/photo.png)`) should
  use forward slashes — they're URI-style references resolved by
  the asset resolver, not OS paths.

## Option D: build inside a container

Useful when you don't want a Rust toolchain on your host, or when
your CI hands a writer a containerised build environment. Works
identically on Linux (Podman or Docker), macOS (Docker Desktop /
Podman Machine / OrbStack), and Windows (Docker Desktop / Podman
Machine).

### One-shot build

From the parent folder containing `markdoc/`, `flux-types/` and
`markdoc-pdf/` side-by-side:

```sh
podman run --rm -v "$PWD":/work -w /work/markdoc-pdf \
    docker.io/library/rust:1.93-alpine \
    sh -c 'apk add --no-cache musl-dev && cargo build --release --bin markdoc-pdf'
# Binary lands at: markdoc-pdf/target/release/markdoc-pdf
```

(Replace `podman` with `docker` if that's what you have.)

### Reusable dev container

Build once, then drop into a shell with the toolchain ready:

```sh
podman build -t markdoc-pdf-dev - <<'EOF'
FROM docker.io/library/rust:1.93-alpine
RUN apk add --no-cache musl-dev pkgconfig openssl-dev git
WORKDIR /work
EOF

# From the parent folder containing the three sibling repos:
podman run --rm -it -v "$PWD":/work -w /work/markdoc-pdf \
    markdoc-pdf-dev sh
# inside the container:
cargo build --release --bin markdoc-pdf
exit
```

### Render docs without a host install

Run `markdoc-pdf` itself inside the container — useful for one-off
renders on a machine where you don't want to install anything:

```sh
podman run --rm -v "$PWD":/work -w /work \
    docker.io/library/rust:1.93-alpine \
    sh -c 'apk add --no-cache musl-dev >/dev/null \
           && cd markdoc-pdf && cargo build --release --bin markdoc-pdf 2>/dev/null \
           && ./target/release/markdoc-pdf -i ../my-doc/intro.mdoc -o /work/intro.pdf -s examples/themes/letter.style.toml'
```

Slow (rebuilds every run); the reusable dev container above is
better for iteration.

## Option E: pre-built binaries

Available as GitHub release artefacts once a tagged release lands.
The CI workflow at `.github/workflows/release.yml` builds binaries
for **Linux x86_64**, **macOS arm64** and **Windows x86_64** on
tag pushes — see the repository's Releases page for the latest.

## Updating

Source-installed (Option A): re-run `cargo install --path . --locked`
from the repo. Cargo notices when the source has changed and rebuilds.

Binary-copied (Option B): rebuild and copy again.

## Uninstalling

- Cargo install: `cargo uninstall markdoc-pdf`
- Manual copy: `sudo rm /usr/local/bin/markdoc-pdf`

## Themes

Themes live in `markdoc-pdf/examples/themes/`. They are not bundled
into the binary — pass the path explicitly via `--style`:

```sh
markdoc-pdf -i my.mdoc -o my.pdf -s /path/to/themes/letter.style.toml
```

If you want a stable location for themes on a writer's machine, copy
the whole `themes/` folder somewhere persistent (e.g. `~/.config/markdoc-themes/`)
and reference it from there:

```sh
mkdir -p ~/.config/markdoc-themes
cp -r markdoc-pdf/examples/themes/* ~/.config/markdoc-themes/
markdoc-pdf -i my.mdoc -o my.pdf -s ~/.config/markdoc-themes/letter.style.toml
```

## Troubleshooting

**"command not found: markdoc-pdf"** — the binary isn't on `$PATH`.
For Option A check `~/.cargo/bin` is on `$PATH`; for Option B check
the directory you copied to. On Windows, restart your shell after
the rustup installer finishes so the PATH change takes effect.

**Windows: `link.exe not found`** — the MSVC toolchain needs the
Visual Studio Build Tools. Re-run the rustup installer and let it
install them, or grab the standalone "Build Tools for Visual Studio"
from Microsoft.

**Compile errors during `cargo install`** — Rust toolchain too old.
Update with `rustup update stable`.

**SQLX_OFFLINE warnings during build** — only relevant when building
sibling crates that talk to a database. `markdoc-pdf` itself has no
database dependency; you can ignore the warnings.

**Fonts look wrong / missing characters** — the bundled Noto Sans
family covers Latin / Cyrillic / Greek / Arabic / Devanagari plus
emoji. For other scripts or branded fonts, use `font_paths` and
`body_font_families` in your style file (see `examples/themes/README.md`).
