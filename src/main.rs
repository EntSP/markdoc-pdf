//! markdoc-pdf — render a Markdoc source file to PDF.
//!
//! Designed for tech writers iterating locally. Errors are printed
//! plainly to stderr with the file path that caused them and (where
//! possible) a hint about how to fix it.

use clap::Parser;
use flux_types::FluxFrontmatter;
use markdoc::{
    Context, evaluate_conditionals, parse,
    partials::{FsPartialResolver, expand_partials},
    resolve_crossrefs, transform_with_context,
    types::{Config, Scalar},
};
use markdoc_pdf::assets::FsAssetResolver;
use markdoc_pdf::dates;
use markdoc_pdf::render::{RenderContext, Style, render_pdf_with};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "markdoc-pdf",
    author,
    version,
    about = "Render a Markdoc source file (.mdoc) to PDF.",
    long_about = "Render a Markdoc source file (.mdoc) to PDF.\n\n\
                  Frontmatter (title / authors / language / firstReleaseDate / …) \
                  drives the PDF metadata and the {title}/{date}/etc. header-footer \
                  template variables. Pass --style to pick from the built-in theme \
                  catalogue or supply your own .style.toml."
)]
struct Args {
    /// Path to the input `.mdoc` file.
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Where to write the output PDF.
    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    /// Optional style file (TOML). Examples ship in
    /// `markdoc-pdf/examples/themes/`; passing none uses the built-in
    /// default style (A4, Noto Sans, no decoration).
    #[arg(short, long, value_name = "FILE")]
    style: Option<PathBuf>,

    /// Root directory used to resolve relative `<img>` and
    /// `{% media %}` `src` attributes. Defaults to the input file's
    /// parent directory — fits the typical layout of one folder per
    /// document with media siblings.
    #[arg(long, value_name = "DIR")]
    assets_root: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            for hint in e.hints() {
                eprintln!("hint:  {hint}");
            }
            ExitCode::from(1)
        }
    }
}

fn run(args: &Args) -> Result<(), AppError> {
    // ── Validate inputs early so we can give path-aware errors. ────
    if !args.input.exists() {
        return Err(AppError::InputMissing(args.input.clone()));
    }
    if let Some(s) = &args.style
        && !s.exists()
    {
        return Err(AppError::StyleMissing(s.clone()));
    }
    if let Some(parent) = args.output.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(AppError::OutputDirMissing(parent.to_path_buf()));
    }
    let assets_root = args
        .assets_root
        .clone()
        .or_else(|| args.input.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    if !assets_root.exists() {
        return Err(AppError::AssetsRootMissing(assets_root));
    }

    // ── Load style. ────────────────────────────────────────────────
    let style = match args.style.as_ref() {
        Some(path) => Style::from_toml_file(path)
            .map_err(|e| AppError::StyleLoad(path.clone(), e.to_string()))?,
        None => Style::default(),
    };

    // ── Read + parse the source. ───────────────────────────────────
    let source =
        fs::read_to_string(&args.input).map_err(|e| AppError::Read(args.input.clone(), e))?;
    let doc =
        parse(&source, None).map_err(|e| AppError::Parse(args.input.clone(), e.to_string()))?;

    // Expand `{% partial file="..." /%}` references against the input
    // file's parent directory. Partials' own `{% partial %}` tags are
    // resolved recursively; cycles are detected and reported.
    let partial_root = args
        .input
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let partial_resolver = FsPartialResolver::new(partial_root);
    let doc = expand_partials(&doc, &partial_resolver)
        .map_err(|e| AppError::Partials(args.input.clone(), e.to_string()))?;

    let doc = resolve_crossrefs(&doc);
    let ctx = Context::new();
    let doc = evaluate_conditionals(&doc, &ctx)
        .map_err(|e| AppError::Conditionals(args.input.clone(), e.to_string()))?;
    let rendered = transform_with_context(&doc, &Config::default(), &ctx)
        .map_err(|e| AppError::Transform(args.input.clone(), e.to_string()))?;

    let resolver = FsAssetResolver::new(&assets_root);

    // ── Pull doc metadata from frontmatter. A document with no YAML
    //    block is fine (fields fall back to defaults), but frontmatter
    //    that is present yet unparsable is surfaced as a warning rather
    //    than silently dropped — otherwise a single bad field would
    //    quietly strip the title, dates and everything else.
    let fm_opt = match FluxFrontmatter::from_node(&doc) {
        Ok(fm) => Some(fm),
        Err(flux_types::FluxError::NoFrontmatter) => None,
        Err(e) => {
            eprintln!(
                "warning: ignoring unparsable frontmatter in {} ({e}); \
                 title/date metadata may be missing",
                args.input.display()
            );
            None
        }
    };
    let creation_date = fm_opt
        .as_ref()
        .and_then(|fm| fm.first_release_date.as_deref())
        .and_then(dates::parse_iso)
        .or_else(|| Some(dates::now()));
    let authors = fm_opt
        .as_ref()
        .map(|fm| fm.authors.clone())
        .unwrap_or_default();
    let creator = fm_opt
        .as_ref()
        .and_then(|fm| fm.creator.clone())
        .or_else(|| Some("markdoc-pdf".to_string()));
    let date_string = fm_opt
        .as_ref()
        .and_then(|fm| fm.first_release_date.as_deref())
        .map(dates::iso_to_date_only);

    // Expose the document's own frontmatter as template variables for
    // header/footer and cover-page templates (`{version}`, `{language}`,
    // … — whatever the author wrote). The renderer stays domain-agnostic:
    // every scalar frontmatter field is surfaced under its authored key,
    // with no hard-coded field names here. Unset fields simply never
    // appear, so their `{name}` token stays literal (and detail lines
    // that resolve to nothing are skipped).
    let mut vars: HashMap<String, String> = HashMap::new();
    collect_frontmatter_vars(&doc, &mut vars);
    // Pre-compute the copyright year span (a generic derived value, not a
    // frontmatter field): a single year when the first-release year
    // equals — or is missing for — the current year, otherwise
    // `first–current`.
    let first_year = fm_opt
        .as_ref()
        .and_then(|fm| fm.first_release_date.as_deref())
        .and_then(dates::year_of);
    vars.insert(
        "copyright_years".to_string(),
        dates::copyright_year_span(first_year, dates::current_year()),
    );

    let render_ctx = RenderContext {
        title: fm_opt
            .as_ref()
            .and_then(|fm| fm.title.clone())
            .unwrap_or_default(),
        language: fm_opt.as_ref().and_then(|fm| fm.language.clone()),
        description: fm_opt.as_ref().and_then(|fm| fm.description.clone()),
        authors,
        creator,
        producer: Some(format!("markdoc-pdf {}", env!("CARGO_PKG_VERSION"))),
        creation_date,
        date_string,
        vars,
    };

    let pdf = render_pdf_with(&rendered, &style, &resolver, &render_ctx);

    fs::write(&args.output, &pdf).map_err(|e| AppError::Write(args.output.clone(), e))?;
    eprintln!(
        "wrote {} ({} bytes) from {}",
        args.output.display(),
        pdf.len(),
        args.input.display()
    );
    Ok(())
}

/// Surface every scalar frontmatter field as a template variable keyed
/// by its authored name. Nested values (arrays / objects) and empty
/// strings are skipped — they have no single sensible string form. The
/// renderer never sees the field *names*, so it stays domain-agnostic:
/// whatever the document declares (`version`, `language`, a custom
/// `productLine`, …) becomes referenceable as `{name}`.
fn collect_frontmatter_vars(doc: &markdoc::ast::Node, vars: &mut HashMap<String, String>) {
    let Some(Scalar::Object(map)) = doc.attributes.get("frontmatter") else {
        return;
    };
    for (key, value) in map {
        if let Some(s) = scalar_to_template_string(value) {
            vars.insert(key.clone(), s);
        }
    }
}

/// Render a scalar as a flat template string. Whole-number floats print
/// without a fractional part (YAML integers arrive as `f64`). Non-scalar
/// or empty values yield `None`.
fn scalar_to_template_string(value: &Scalar) -> Option<String> {
    match value {
        Scalar::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Scalar::String(_) => None,
        Scalar::Number(n) if n.is_finite() && n.fract() == 0.0 => Some(format!("{}", *n as i64)),
        Scalar::Number(n) => Some(n.to_string()),
        Scalar::Boolean(b) => Some(b.to_string()),
        Scalar::Null | Scalar::Array(_) | Scalar::Object(_) => None,
    }
}

/// Writer-facing error type. Each variant carries the file path that
/// caused it (where applicable) and produces a one-line message; the
/// `hints` method returns optional follow-up advice that's printed
/// after the error.
#[derive(Debug)]
enum AppError {
    InputMissing(PathBuf),
    StyleMissing(PathBuf),
    OutputDirMissing(PathBuf),
    AssetsRootMissing(PathBuf),
    StyleLoad(PathBuf, String),
    Read(PathBuf, std::io::Error),
    Write(PathBuf, std::io::Error),
    Parse(PathBuf, String),
    Partials(PathBuf, String),
    Conditionals(PathBuf, String),
    Transform(PathBuf, String),
}

impl AppError {
    fn hints(&self) -> Vec<String> {
        match self {
            AppError::InputMissing(_) => vec![
                "check the path is right (relative paths are resolved against the current directory).".into(),
            ],
            AppError::StyleMissing(_) => vec![
                "see markdoc-pdf/examples/themes/ for ready-made styles, or omit --style for the built-in default.".into(),
            ],
            AppError::OutputDirMissing(p) => vec![
                format!("create the directory first: mkdir -p {}", p.display()),
            ],
            AppError::AssetsRootMissing(_) => vec![
                "--assets-root must point at an existing directory; defaults to the input file's parent.".into(),
            ],
            AppError::StyleLoad(_, msg) if msg.contains("missing field") => vec![
                "the style file is missing a required field — start from one of the example themes and tweak.".into(),
            ],
            AppError::StyleLoad(_, _) => vec![
                "if you're new to the style format, copy markdoc-pdf/examples/themes/letter.style.toml and edit.".into(),
            ],
            AppError::Parse(_, _) => vec![
                "Markdoc parses CommonMark plus {% tag %} extensions. Common gotchas: unbalanced tags, missing /%} on self-closing tags.".into(),
            ],
            AppError::Partials(_, msg) if msg.contains("cycle") => vec![
                "two or more partials include each other transitively — break the loop or extract the shared content into a third file.".into(),
            ],
            AppError::Partials(_, _) => vec![
                "partial paths are resolved against the input file's directory; check spelling and that the file exists.".into(),
            ],
            AppError::Conditionals(_, _) => vec![
                "{% if expr %} branches must reference variables defined in frontmatter or via Context.".into(),
            ],
            AppError::Transform(_, _) => vec![
                "transform errors usually mean a tag failed schema validation — check the spelling and required attributes.".into(),
            ],
            _ => Vec::new(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::InputMissing(p) => write!(f, "input file not found: {}", p.display()),
            AppError::StyleMissing(p) => write!(f, "style file not found: {}", p.display()),
            AppError::OutputDirMissing(p) => {
                write!(f, "output directory does not exist: {}", p.display())
            }
            AppError::AssetsRootMissing(p) => {
                write!(f, "--assets-root does not exist: {}", p.display())
            }
            AppError::StyleLoad(p, msg) => {
                write!(f, "couldn't load style {}: {msg}", p.display())
            }
            AppError::Read(p, e) => write!(f, "couldn't read {}: {e}", p.display()),
            AppError::Write(p, e) => write!(f, "couldn't write {}: {e}", p.display()),
            AppError::Parse(p, e) => write!(f, "couldn't parse {}: {e}", p.display()),
            AppError::Partials(p, e) => {
                write!(f, "partial expansion failed for {}: {e}", p.display())
            }
            AppError::Conditionals(p, e) => {
                write!(f, "conditional evaluation failed in {}: {e}", p.display())
            }
            AppError::Transform(p, e) => {
                write!(f, "transform failed in {}: {e}", p.display())
            }
        }
    }
}

impl std::error::Error for AppError {}

// `Path` import suppression — only used in trait bound.
#[allow(dead_code)]
fn _unused(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_strings_and_bools() {
        assert_eq!(
            scalar_to_template_string(&Scalar::String("6.1".into())).as_deref(),
            Some("6.1")
        );
        // Blank / whitespace-only strings are dropped.
        assert_eq!(
            scalar_to_template_string(&Scalar::String("  ".into())),
            None
        );
        assert_eq!(
            scalar_to_template_string(&Scalar::Boolean(true)).as_deref(),
            Some("true")
        );
    }

    #[test]
    fn whole_number_floats_print_as_integers() {
        // YAML integers arrive as f64 — render `75371702`, not `75371702.0`.
        assert_eq!(
            scalar_to_template_string(&Scalar::Number(75371702.0)).as_deref(),
            Some("75371702")
        );
        assert_eq!(
            scalar_to_template_string(&Scalar::Number(2.5)).as_deref(),
            Some("2.5")
        );
    }

    #[test]
    fn non_scalar_values_are_skipped() {
        assert_eq!(scalar_to_template_string(&Scalar::Null), None);
        assert_eq!(scalar_to_template_string(&Scalar::Array(vec![])), None);
        assert_eq!(
            scalar_to_template_string(&Scalar::Object(std::collections::HashMap::new())),
            None
        );
    }
}
