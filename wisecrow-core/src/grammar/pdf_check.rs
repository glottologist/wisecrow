//! Screens a PDF before it is imported as grammar material.
//!
//! A grammar book is only useful to us if its text layer can be read. A PDF
//! built from page scans holds no text at all, an encrypted one cannot be opened,
//! and a reference work that is mostly paradigm tables yields prose too sparse to
//! reason from. Each of those failures is cheap to detect and expensive to
//! discover halfway through an import, so this module reads a file exactly as
//! [`crate::grammar::pdf`] does -- the native extractor first, poppler where it
//! comes back thin -- and reports what an import would find, naming which
//! extractor answered.
//!
//! The report is deliberately infallible: a batch of candidate documents is
//! screened in one run, and a file that cannot be opened is a verdict about that
//! file rather than an error that abandons the others.

use std::fmt;
use std::path::Path;

use crate::grammar::pdf::{parse_passages, GrammarPassage};
use crate::grammar::pdf_text::{Extractor, MIN_CHARS_PER_PAGE};

/// Bytes of a file inspected for the PDF signature.
///
/// The signature is expected at the start, but a file served with a preamble
/// still opens, so we look as far into the head as the web upload's own check.
const HEADER_WINDOW: usize = 1024;
const PDF_HEADER: &[u8] = b"%PDF-";
/// Passages quoted in the report, so that a verdict can be judged by eye.
const SAMPLES: usize = 3;
/// Characters shown of a sampled passage.
const SAMPLE_CHARS: usize = 110;
const BYTES_PER_MEGABYTE: f64 = 1_048_576.0;

/// Whether a document can supply grammar material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The text layer holds prose at the density of a grammar book.
    Usable,
    /// Text was extracted, but too little of it to reason from.
    Thin,
    /// Nothing can be read from the file, for the reason given.
    Unusable(String),
}

impl Verdict {
    /// Reports whether an import of this document is worth attempting.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        matches!(*self, Self::Usable)
    }

    /// The four-character label the report line opens with.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match *self {
            Self::Usable => "OK  ",
            Self::Thin => "THIN",
            Self::Unusable(_) => "FAIL",
        }
    }
}

/// What one candidate document holds.
#[derive(Debug, Clone)]
pub struct PdfReport {
    /// The path as it was given on the command line.
    pub path: String,
    pub bytes: u64,
    pub pages: usize,
    pub encrypted: bool,
    /// Which extractor produced the text, or `None` when none could.
    pub extractor: Option<Extractor>,
    pub characters: usize,
    pub passages: usize,
    pub examples: usize,
    /// Mean length of a kept passage, in characters.
    pub mean_passage_chars: usize,
    pub verdict: Verdict,
    /// Opening lines of the first few passages, headings included.
    pub samples: Vec<String>,
}

impl PdfReport {
    /// Characters of text per page of document.
    #[must_use]
    pub fn chars_per_page(&self) -> usize {
        self.characters.checked_div(self.pages).unwrap_or(0)
    }
}

impl fmt::Display for PdfReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let megabytes = self.bytes as f64 / BYTES_PER_MEGABYTE;
        writeln!(formatter, "{} {}", self.verdict.label(), self.path)?;
        if self.pages == 0 {
            let reason = match self.verdict {
                Verdict::Unusable(ref reason) => reason.as_str(),
                _ => "holds nothing that could be read",
            };
            writeln!(formatter, "  {megabytes:.1} MB; {reason}")?;
            return Ok(());
        }
        let encryption = if self.encrypted {
            "encrypted"
        } else {
            "not encrypted"
        };
        let read_by = self.extractor.map_or_else(String::new, |extractor| {
            format!(", read by {}", extractor.label())
        });
        writeln!(
            formatter,
            "  {megabytes:.1} MB, {} pages, {encryption}{read_by}",
            self.pages
        )?;
        writeln!(
            formatter,
            "  {} chars over {} pages -> {} chars/page",
            self.characters,
            self.pages,
            self.chars_per_page()
        )?;
        writeln!(
            formatter,
            "  {} passages, mean {} chars, {} examples",
            self.passages, self.mean_passage_chars, self.examples
        )?;
        if let Verdict::Unusable(ref reason) = self.verdict {
            writeln!(formatter, "  {reason}")?;
        }
        for sample in &self.samples {
            writeln!(formatter, "    | {sample}")?;
        }
        Ok(())
    }
}

/// Reports what an import would find in the PDF at `path`.
///
/// Every failure is carried in the report's [`Verdict`] rather than returned, so
/// that a run over several candidates reports on all of them.
#[must_use]
pub fn check(path: &Path) -> PdfReport {
    let display = path.display().to_string();
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(error) => return unusable(display, 0, format!("cannot be read: {error}")),
    };
    let bytes = u64::try_from(data.len()).unwrap_or(u64::MAX);

    if !has_pdf_header(&data) {
        return unusable(
            display,
            bytes,
            "does not begin with a PDF signature".to_owned(),
        );
    }

    let document = match pdf_extract::Document::load_mem(&data) {
        Ok(document) => document,
        Err(error) => return unusable(display, bytes, format!("will not open: {error}")),
    };
    let encrypted = document.is_encrypted();
    let pages = document.get_pages().len();
    if encrypted {
        return unusable(
            display,
            bytes,
            "is encrypted, so no text can be extracted".to_owned(),
        );
    }
    if pages == 0 {
        return unusable(display, bytes, "holds no pages".to_owned());
    }

    let text = match crate::grammar::pdf_text::pages(path) {
        Ok(text) => text,
        Err(error) => return unusable(display, bytes, error.to_string()),
    };
    let characters: usize = text
        .pages
        .iter()
        .map(|page| page.chars().count())
        .fold(0, usize::saturating_add);

    let passages = parse_passages(&text.pages);
    let passage_chars: usize = passages
        .iter()
        .map(|passage| passage.text.chars().count())
        .fold(0, usize::saturating_add);
    let examples = passages
        .iter()
        .map(|passage| passage.examples.len())
        .fold(0, usize::saturating_add);
    let samples = sample_lines(&passages);

    let mut report = PdfReport {
        path: display,
        bytes,
        pages,
        encrypted,
        extractor: Some(text.extractor),
        characters,
        passages: passages.len(),
        examples,
        mean_passage_chars: passage_chars.checked_div(passages.len()).unwrap_or(0),
        verdict: Verdict::Usable,
        samples,
    };
    report.verdict = classify(report.chars_per_page(), report.passages);
    report
}

fn classify(chars_per_page: usize, passages: usize) -> Verdict {
    if passages == 0 {
        return Verdict::Unusable(
            "holds no passage of prose long enough to state a rule".to_owned(),
        );
    }
    if chars_per_page < MIN_CHARS_PER_PAGE {
        return Verdict::Thin;
    }
    Verdict::Usable
}

fn has_pdf_header(data: &[u8]) -> bool {
    let head = data.get(..data.len().min(HEADER_WINDOW)).unwrap_or(data);
    head.windows(PDF_HEADER.len())
        .any(|window| window == PDF_HEADER)
}

/// Quotes passages from across the document rather than from its opening pages.
///
/// The front of a grammar book is its title page and its table of contents, so a
/// sample taken from there tells us nothing about the prose an import would read.
/// The quotations are spread over the length of the document instead, which is
/// also how [`crate::grammar::pdf_import`] spends its passage budget.
fn sample_lines(passages: &[GrammarPassage]) -> Vec<String> {
    let mut chosen: Vec<usize> = (1..=SAMPLES)
        .filter_map(|nth| {
            passages
                .len()
                .checked_mul(nth)
                .and_then(|scaled| scaled.checked_div(SAMPLES.saturating_add(1)))
        })
        .collect();
    chosen.dedup();
    chosen
        .iter()
        .filter_map(|index| passages.get(*index))
        .map(sample_line)
        .collect()
}

fn sample_line(passage: &GrammarPassage) -> String {
    let heading = passage.heading.as_deref().unwrap_or("(no heading)");
    let words: Vec<&str> = passage.text.split_whitespace().collect();
    let opening: String = words.join(" ").chars().take(SAMPLE_CHARS).collect();
    format!("p.{} {heading}: {opening}", passage.page)
}

fn unusable(path: String, bytes: u64, reason: String) -> PdfReport {
    PdfReport {
        path,
        bytes,
        pages: 0,
        encrypted: false,
        extractor: None,
        characters: 0,
        passages: 0,
        examples: 0,
        mean_passage_chars: 0,
        verdict: Verdict::Unusable(reason),
        samples: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn report(verdict: Verdict) -> PdfReport {
        PdfReport {
            path: "grammar/example.pdf".to_owned(),
            bytes: 3_355_443,
            pages: 214,
            encrypted: false,
            extractor: Some(Extractor::Native),
            characters: 412_093,
            passages: 186,
            examples: 92,
            mean_passage_chars: 503,
            verdict,
            samples: vec!["p.12 Present Tense: Regular verbs take the endings".to_owned()],
        }
    }

    #[rstest]
    #[case(1926, 186, Verdict::Usable)]
    #[case(MIN_CHARS_PER_PAGE, 1, Verdict::Usable)]
    #[case(199, 4, Verdict::Thin)]
    #[case(0, 1, Verdict::Thin)]
    fn density_and_passages_decide_the_verdict(
        #[case] chars_per_page: usize,
        #[case] passages: usize,
        #[case] expected: Verdict,
    ) {
        assert_eq!(classify(chars_per_page, passages), expected);
    }

    #[test]
    fn a_document_without_passages_is_unusable_however_dense() {
        assert!(matches!(classify(4000, 0), Verdict::Unusable(_)));
    }

    #[rstest]
    #[case(&b"%PDF-1.7\nnonsense"[..], true)]
    #[case(&b"\n\n%PDF-1.4"[..], true)]
    #[case(&b"<!DOCTYPE html><html>"[..], false)]
    #[case(&b""[..], false)]
    fn pdf_signature_detected(#[case] data: &[u8], #[case] expected: bool) {
        assert_eq!(has_pdf_header(data), expected);
    }

    #[test]
    fn a_signature_past_the_header_window_is_not_a_pdf() {
        let mut data = vec![b' '; HEADER_WINDOW];
        data.extend_from_slice(PDF_HEADER);
        assert!(!has_pdf_header(&data));
    }

    #[test]
    fn a_missing_file_is_reported_rather_than_returned() {
        let checked = check(Path::new("grammar/no-such-document.pdf"));
        assert!(matches!(checked.verdict, Verdict::Unusable(_)));
        assert_eq!(checked.bytes, 0);
    }

    #[test]
    fn a_file_that_is_not_a_pdf_is_refused_before_extraction() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("prose.txt");
        std::fs::write(&path, "This is not a PDF at all.").expect("write candidate");

        let checked = check(&path);

        match checked.verdict {
            Verdict::Unusable(reason) => assert!(reason.contains("PDF signature"), "{reason}"),
            other => panic!("expected an unusable verdict, got {other:?}"),
        }
    }

    #[test]
    fn a_usable_report_names_density_passages_and_samples() {
        let rendered = report(Verdict::Usable).to_string();
        assert!(
            rendered.starts_with("OK   grammar/example.pdf\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("3.2 MB, 214 pages, not encrypted"),
            "{rendered}"
        );
        assert!(
            rendered.contains("412093 chars over 214 pages -> 1925 chars/page"),
            "{rendered}"
        );
        assert!(
            rendered.contains("186 passages, mean 503 chars, 92 examples"),
            "{rendered}"
        );
        assert!(rendered.contains("    | p.12 Present Tense:"), "{rendered}");
    }

    #[test]
    fn a_document_that_never_opened_gives_the_reason_and_nothing_else() {
        let mut unopened = report(Verdict::Unusable("is encrypted".to_owned()));
        unopened.pages = 0;

        let rendered = unopened.to_string();

        assert!(
            rendered.starts_with("FAIL grammar/example.pdf\n"),
            "{rendered}"
        );
        assert!(rendered.contains("is encrypted"), "{rendered}");
        assert!(!rendered.contains("chars/page"), "{rendered}");
    }

    /// The density is the evidence that separates a page scan from a book of
    /// exercises, so a document that was read and then rejected must still show it.
    #[test]
    fn a_document_read_and_then_rejected_still_reports_its_density() {
        let mut rejected = report(Verdict::Unusable(
            "holds no passage of prose long enough to state a rule".to_owned(),
        ));
        rejected.passages = 0;
        rejected.mean_passage_chars = 0;
        rejected.examples = 0;
        rejected.samples = Vec::new();

        let rendered = rejected.to_string();

        assert!(
            rendered.contains("412093 chars over 214 pages -> 1925 chars/page"),
            "{rendered}"
        );
        assert!(
            rendered.contains("holds no passage of prose long enough"),
            "{rendered}"
        );
    }

    fn passage(page: usize) -> GrammarPassage {
        GrammarPassage {
            heading: Some(format!("Section {page}")),
            text: format!("Prose from page {page}."),
            page,
            examples: Vec::new(),
        }
    }

    #[test]
    fn samples_are_taken_from_across_the_document() {
        let passages: Vec<GrammarPassage> = (1..=12).map(passage).collect();

        let samples = sample_lines(&passages);

        assert_eq!(samples.len(), SAMPLES);
        assert!(samples[0].starts_with("p.4 "), "{samples:?}");
        assert!(samples[1].starts_with("p.7 "), "{samples:?}");
        assert!(samples[2].starts_with("p.10 "), "{samples:?}");
    }

    #[test]
    fn a_short_document_is_not_quoted_three_times_over() {
        let samples = sample_lines(&[passage(1)]);
        assert_eq!(samples.len(), 1, "{samples:?}");
    }

    #[test]
    fn sample_lines_collapse_the_text_layer_whitespace() {
        let passage = GrammarPassage {
            heading: Some("Present Tense".to_owned()),
            text: "Regular   verbs\n   take the endings".to_owned(),
            page: 12,
            examples: Vec::new(),
        };
        assert_eq!(
            sample_line(&passage),
            "p.12 Present Tense: Regular verbs take the endings"
        );
    }
}
