use crate::errors::WisecrowError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::fs::File;
use std::io::BufReader;
use tokio::sync::mpsc::Sender;

#[derive(Debug, PartialEq, Eq)]
pub struct TranslationPair {
    pub source_text: String,
    pub target_text: String,
}

trait XmlParseHandler {
    fn on_start(&mut self, e: &BytesStart<'_>);
    fn is_in_text(&self) -> bool;
    fn text_buffer(&mut self) -> &mut String;
    async fn on_end(
        &mut self,
        name: &[u8],
        source_lang: &str,
        target_lang: &str,
        sender: &Sender<TranslationPair>,
        count: &mut usize,
    ) -> bool;
    fn format_label(&self) -> &'static str;
}

struct TmxState {
    in_seg: bool,
    seg_buffer: String,
    current_lang: Option<String>,
    source_text: Option<String>,
    target_text: Option<String>,
}

impl TmxState {
    const fn new() -> Self {
        Self {
            in_seg: false,
            seg_buffer: String::new(),
            current_lang: None,
            source_text: None,
            target_text: None,
        }
    }
}

impl XmlParseHandler for TmxState {
    fn on_start(&mut self, e: &BytesStart<'_>) {
        match e.name().as_ref() {
            b"tu" => {
                self.source_text = None;
                self.target_text = None;
            }
            b"tuv" => self.current_lang = CorpusParser::read_lang_attr(e),
            b"seg" => {
                self.in_seg = true;
                self.seg_buffer.clear();
            }
            _ => {}
        }
    }

    fn is_in_text(&self) -> bool {
        self.in_seg
    }

    fn text_buffer(&mut self) -> &mut String {
        &mut self.seg_buffer
    }

    async fn on_end(
        &mut self,
        name: &[u8],
        source_lang: &str,
        target_lang: &str,
        sender: &Sender<TranslationPair>,
        count: &mut usize,
    ) -> bool {
        match name {
            b"seg" => {
                self.in_seg = false;
                CorpusParser::assign_by_lang(
                    self.current_lang.as_deref(),
                    source_lang,
                    target_lang,
                    &mut self.seg_buffer,
                    &mut self.source_text,
                    &mut self.target_text,
                );
            }
            b"tuv" => self.current_lang = None,
            b"tu" => {
                if !CorpusParser::send_pair(
                    &mut self.source_text,
                    &mut self.target_text,
                    sender,
                    count,
                )
                .await
                {
                    return false;
                }
                if *count > 0 && *count % 1000 == 0 {
                    tracing::info!("Parsed {count} TMX pairs");
                }
            }
            _ => {}
        }
        true
    }

    fn format_label(&self) -> &'static str {
        "TMX"
    }
}

struct XmlState {
    in_link_grp: bool,
    in_s: bool,
    s_buffer: String,
    current_lang: Option<String>,
    source_text: Option<String>,
    target_text: Option<String>,
}

impl XmlState {
    const fn new() -> Self {
        Self {
            in_link_grp: false,
            in_s: false,
            s_buffer: String::new(),
            current_lang: None,
            source_text: None,
            target_text: None,
        }
    }
}

impl XmlParseHandler for XmlState {
    fn on_start(&mut self, e: &BytesStart<'_>) {
        match e.name().as_ref() {
            b"linkGrp" => self.in_link_grp = true,
            b"s" if self.in_link_grp => {
                self.current_lang = CorpusParser::read_lang_attr(e);
                self.in_s = true;
                self.s_buffer.clear();
            }
            _ => {}
        }
    }

    fn is_in_text(&self) -> bool {
        self.in_s
    }

    fn text_buffer(&mut self) -> &mut String {
        &mut self.s_buffer
    }

    async fn on_end(
        &mut self,
        name: &[u8],
        source_lang: &str,
        target_lang: &str,
        sender: &Sender<TranslationPair>,
        count: &mut usize,
    ) -> bool {
        match name {
            b"s" if self.in_s => {
                self.in_s = false;
                CorpusParser::assign_by_lang(
                    self.current_lang.as_deref(),
                    source_lang,
                    target_lang,
                    &mut self.s_buffer,
                    &mut self.source_text,
                    &mut self.target_text,
                );
                if self.source_text.is_some() && self.target_text.is_some() {
                    if !CorpusParser::send_pair(
                        &mut self.source_text,
                        &mut self.target_text,
                        sender,
                        count,
                    )
                    .await
                    {
                        return false;
                    }
                    if *count > 0 && *count % 1000 == 0 {
                        tracing::info!("Parsed {count} XML alignment pairs");
                    }
                }
            }
            b"linkGrp" => {
                self.in_link_grp = false;
                self.source_text = None;
                self.target_text = None;
            }
            _ => {}
        }
        true
    }

    fn format_label(&self) -> &'static str {
        "XML alignment"
    }
}

pub struct CorpusParser;

impl CorpusParser {
    fn read_lang_attr(e: &BytesStart<'_>) -> Option<String> {
        for name in &["xml:lang", "lang"] {
            if let Ok(Some(attr)) = e.try_get_attribute(*name) {
                if let Ok(val) = std::str::from_utf8(&attr.value) {
                    return Some(val.to_owned());
                }
            }
        }
        None
    }

    fn assign_by_lang(
        lang: Option<&str>,
        source_lang: &str,
        target_lang: &str,
        buffer: &mut String,
        source: &mut Option<String>,
        target: &mut Option<String>,
    ) {
        match lang {
            Some(l) if l == source_lang => *source = Some(std::mem::take(buffer)),
            Some(l) if l == target_lang => *target = Some(std::mem::take(buffer)),
            _ => {}
        }
    }

    /// Removes NUL characters from a phrase. PostgreSQL cannot store `U+0000`
    /// in a `text` column at all, and web-mined corpora do carry them: a single
    /// stray byte in the CCMatrix Gaelic release aborted an entire ingest with
    /// `invalid byte sequence for encoding "UTF8": 0x00`. Stripping is
    /// preferable to skipping the pair, since the rest of the sentence is
    /// perfectly good text.
    fn strip_nuls(text: String) -> String {
        if text.contains('\0') {
            text.replace('\0', "")
        } else {
            text
        }
    }

    /// Mirrors `chk_from_phrase_length` and `chk_to_phrase_length` from
    /// migration `003_performance_indexes.sql`. A pair breaching this is
    /// dropped here rather than at the database, because a rejected row aborts
    /// the whole batch and with it the rest of the corpus — CCAligned Welsh
    /// lost several hundred thousand pairs to one over-long paragraph. Nothing
    /// of value goes with it: deck selection only ever considers phrases
    /// between 2 and 200 characters.
    const MAX_PHRASE_CHARS: usize = 1000;

    fn is_storable(text: &str) -> bool {
        text.chars().count() <= Self::MAX_PHRASE_CHARS
    }

    /// PostgreSQL's version-4 btree cannot index a tuple larger than this.
    const BTREE_MAX_INDEX_TUPLE_BYTES: usize = 2704;

    /// Headroom reserved within a btree tuple for the index tuple header, its
    /// two `int4` language keys and the per-datum varlena headers and
    /// alignment padding that accompany the two phrases.
    const INDEX_TUPLE_OVERHEAD_BYTES: usize = 104;

    /// The unique index `translations_unique_pair` (migration
    /// `005_fix_translation_unique_constraint.sql`) spans both phrases, so the
    /// two `char_length(..) <= 1000` CHECK constraints — which bound each
    /// phrase independently, in characters — do not bound their combined byte
    /// length. A pair of long multi-byte phrases therefore satisfies both
    /// CHECKs and `is_storable` yet overflows the index: CCAligned Welsh
    /// aborted on exactly such a row with `index row size 2752 exceeds btree
    /// version 4 maximum 2704`. Deriving the budget from the btree maximum
    /// keeps the two in step (and an oversized overhead would fail to compile
    /// on const underflow). Nothing usable is lost: deck selection only
    /// considers phrases between 2 and 200 characters.
    const MAX_INDEX_KEY_BYTES: usize =
        Self::BTREE_MAX_INDEX_TUPLE_BYTES - Self::INDEX_TUPLE_OVERHEAD_BYTES;

    fn fits_unique_index(src: &str, tgt: &str) -> bool {
        src.len() + tgt.len() <= Self::MAX_INDEX_KEY_BYTES
    }

    async fn send_pair(
        source: &mut Option<String>,
        target: &mut Option<String>,
        sender: &Sender<TranslationPair>,
        count: &mut usize,
    ) -> bool {
        if let (Some(src), Some(tgt)) = (source.take(), target.take()) {
            let (src, tgt) = (Self::strip_nuls(src), Self::strip_nuls(tgt));
            if src.is_empty()
                || tgt.is_empty()
                || !Self::is_storable(&src)
                || !Self::is_storable(&tgt)
                || !Self::fits_unique_index(&src, &tgt)
            {
                return true;
            }
            let pair = TranslationPair {
                source_text: src,
                target_text: tgt,
            };
            if sender.send(pair).await.is_err() {
                return false;
            }
            *count += 1;
        }
        true
    }

    async fn parse_xml_events<H: XmlParseHandler>(
        path: &str,
        source_lang: &str,
        target_lang: &str,
        sender: &Sender<TranslationPair>,
        handler: &mut H,
    ) -> Result<usize, WisecrowError> {
        let file = File::open(path)?;
        let mut reader = Reader::from_reader(BufReader::new(file));
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut count = 0usize;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => handler.on_start(&e),
                Ok(Event::Text(e)) if handler.is_in_text() => {
                    if let Ok(t) = e.unescape() {
                        handler.text_buffer().push_str(&t);
                    }
                }
                Ok(Event::End(e)) => {
                    if !handler
                        .on_end(
                            e.name().as_ref(),
                            source_lang,
                            target_lang,
                            sender,
                            &mut count,
                        )
                        .await
                    {
                        return Ok(count);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => tracing::warn!("{} parse error: {e}", handler.format_label()),
                _ => {}
            }
            buf.clear();
        }
        tracing::info!(
            "Finished {}: {count} pairs from {path}",
            handler.format_label()
        );
        Ok(count)
    }

    /// Parses a TMX translation memory file, sending each extracted pair to
    /// `sender`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or a fatal I/O error
    /// occurs during reading.
    pub async fn parse_tmx_file(
        path: &str,
        source_lang: &str,
        target_lang: &str,
        sender: &Sender<TranslationPair>,
    ) -> Result<usize, WisecrowError> {
        let mut state = TmxState::new();
        Self::parse_xml_events(path, source_lang, target_lang, sender, &mut state).await
    }

    /// Parses an OPUS XML alignment file, sending each extracted pair to
    /// `sender`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or a fatal I/O error
    /// occurs during reading.
    pub async fn parse_xml_alignment_file(
        path: &str,
        source_lang: &str,
        target_lang: &str,
        sender: &Sender<TranslationPair>,
    ) -> Result<usize, WisecrowError> {
        let mut state = XmlState::new();
        Self::parse_xml_events(path, source_lang, target_lang, sender, &mut state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use tokio::sync::mpsc;

    fn collect_translations(mut rx: mpsc::Receiver<TranslationPair>) -> Vec<TranslationPair> {
        let mut pairs = Vec::new();
        while let Ok(pair) = rx.try_recv() {
            pairs.push(pair);
        }
        pairs
    }

    #[tokio::test]
    async fn nul_bytes_are_stripped_rather_than_aborting_the_ingest() {
        // A single NUL in the CCMatrix Gaelic release aborted a whole ingest
        // with PostgreSQL's `invalid byte sequence for encoding "UTF8": 0x00`.
        let tmx_content = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<tmx version=\"1.4\"><body>\
<tu><tuv xml:lang=\"en\"><seg>Hel\u{0}lo</seg></tuv>\
<tuv xml:lang=\"gd\"><seg>Hal\u{0}o</seg></tuv></tu>\
<tu><tuv xml:lang=\"en\"><seg>\u{0}</seg></tuv>\
<tuv xml:lang=\"gd\"><seg>Ceart</seg></tuv></tu>\
</body></tmx>";

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(tmx_content.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel(100);
        let count = CorpusParser::parse_tmx_file(tmp.path().to_str().unwrap(), "en", "gd", &tx)
            .await
            .unwrap();
        drop(tx);

        assert_eq!(count, 1, "the all-NUL pair is dropped, the usable one kept");
        let pairs = collect_translations(rx);
        assert_eq!(pairs[0].source_text, "Hello");
        assert_eq!(pairs[0].target_text, "Halo");
        assert!(!pairs
            .iter()
            .any(|p| p.source_text.contains('\0') || p.target_text.contains('\0')));
    }

    #[tokio::test]
    async fn over_long_phrases_are_dropped_rather_than_aborting_the_ingest() {
        // CCAligned Welsh lost the remainder of its corpus to one paragraph
        // breaching chk_from_phrase_length.
        let long = "a".repeat(CorpusParser::MAX_PHRASE_CHARS + 1);
        let at_limit = "b".repeat(CorpusParser::MAX_PHRASE_CHARS);
        let tmx_content = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<tmx version=\"1.4\"><body>\
<tu><tuv xml:lang=\"en\"><seg>{long}</seg></tuv>\
<tuv xml:lang=\"cy\"><seg>iawn</seg></tuv></tu>\
<tu><tuv xml:lang=\"en\"><seg>{at_limit}</seg></tuv>\
<tuv xml:lang=\"cy\"><seg>iawn</seg></tuv></tu>\
</body></tmx>"
        );

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(tmx_content.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel(100);
        let count = CorpusParser::parse_tmx_file(tmp.path().to_str().unwrap(), "en", "cy", &tx)
            .await
            .unwrap();
        drop(tx);

        assert_eq!(count, 1, "over-limit dropped, exactly-at-limit kept");
        let pairs = collect_translations(rx);
        assert_eq!(
            pairs[0].source_text.chars().count(),
            CorpusParser::MAX_PHRASE_CHARS
        );
    }

    #[test]
    fn phrase_limit_matches_the_database_constraint() {
        let migration = include_str!("../../../migrations/003_performance_indexes.sql");
        for column in ["from_phrase", "to_phrase"] {
            assert!(
                migration.contains(&format!(
                    "char_length({column}) <= {}",
                    CorpusParser::MAX_PHRASE_CHARS
                )),
                "MAX_PHRASE_CHARS must match the CHECK constraint on {column}"
            );
        }
    }

    #[tokio::test]
    async fn over_long_combined_pairs_are_dropped_rather_than_aborting_the_ingest() {
        // Each phrase clears char_length(..) <= 1000, but together they
        // overflow the composite unique index — the failure that aborted
        // CCAligned Welsh with `index row size 2752 exceeds btree version 4
        // maximum 2704`. "é" is two UTF-8 bytes, so the char counts stay
        // within MAX_PHRASE_CHARS while the byte counts cross the index limit.
        let half_chars = CorpusParser::MAX_INDEX_KEY_BYTES / 4; // per side, at the limit
        let at_limit = "é".repeat(half_chars); // 2 sides * half_chars * 2 bytes = limit
        let over = "é".repeat(half_chars + 1); // two bytes past the budget per side
        let tmx_content = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<tmx version=\"1.4\"><body>\
<tu><tuv xml:lang=\"en\"><seg>{over}</seg></tuv>\
<tuv xml:lang=\"cy\"><seg>{over}</seg></tuv></tu>\
<tu><tuv xml:lang=\"en\"><seg>{at_limit}</seg></tuv>\
<tuv xml:lang=\"cy\"><seg>{at_limit}</seg></tuv></tu>\
</body></tmx>"
        );

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(tmx_content.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel(100);
        let count = CorpusParser::parse_tmx_file(tmp.path().to_str().unwrap(), "en", "cy", &tx)
            .await
            .unwrap();
        drop(tx);

        assert_eq!(count, 1, "combined over-limit dropped, at-limit kept");
        let pairs = collect_translations(rx);
        assert_eq!(
            pairs[0].source_text.len() + pairs[0].target_text.len(),
            CorpusParser::MAX_INDEX_KEY_BYTES,
            "the kept pair sits exactly at the combined byte budget"
        );
    }

    #[test]
    fn guard_tracks_the_composite_unique_index() {
        // The byte budget only matters because migration 005 creates a unique
        // constraint spanning both phrases; its btree tuple is what overflows.
        // The budget-versus-btree-maximum relationship is asserted at compile
        // time above, so it cannot silently drift.
        let migration =
            include_str!("../../../migrations/005_fix_translation_unique_constraint.sql");
        assert!(
            migration.contains("UNIQUE (from_language_id, from_phrase, to_language_id, to_phrase)"),
            "translations_unique_pair must span both phrases for the byte budget to matter"
        );
    }

    #[tokio::test]
    async fn parse_tmx_extracts_pairs() {
        let tmx_content = r#"<?xml version="1.0" encoding="utf-8"?>
<tmx version="1.4">
  <body>
    <tu>
      <tuv xml:lang="en"><seg>Hello</seg></tuv>
      <tuv xml:lang="es"><seg>Hola</seg></tuv>
    </tu>
    <tu>
      <tuv xml:lang="en"><seg>Goodbye</seg></tuv>
      <tuv xml:lang="es"><seg>Adiós</seg></tuv>
    </tu>
  </body>
</tmx>"#;

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(tmx_content.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel(100);

        let count = CorpusParser::parse_tmx_file(tmp.path().to_str().unwrap(), "en", "es", &tx)
            .await
            .unwrap();
        drop(tx);

        assert_eq!(count, 2);
        let pairs = collect_translations(rx);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].source_text, "Hello");
        assert_eq!(pairs[0].target_text, "Hola");
        assert_eq!(pairs[1].source_text, "Goodbye");
        assert_eq!(pairs[1].target_text, "Adiós");
    }

    #[tokio::test]
    async fn parse_tmx_skips_incomplete_units() {
        let tmx_content = r#"<?xml version="1.0"?>
<tmx version="1.4"><body>
  <tu><tuv xml:lang="en"><seg>Only source</seg></tuv></tu>
  <tu>
    <tuv xml:lang="en"><seg>Has both</seg></tuv>
    <tuv xml:lang="es"><seg>Tiene ambos</seg></tuv>
  </tu>
</body></tmx>"#;

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(tmx_content.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel(100);

        let count = CorpusParser::parse_tmx_file(tmp.path().to_str().unwrap(), "en", "es", &tx)
            .await
            .unwrap();
        drop(tx);

        assert_eq!(count, 1);
        let pairs = collect_translations(rx);
        assert_eq!(pairs[0].source_text, "Has both");
    }

    #[tokio::test]
    async fn parse_xml_alignment_extracts_pairs() {
        let xml_content = r#"<?xml version="1.0" encoding="utf-8"?>
<cesAlign>
  <linkGrp>
    <s xml:lang="en">Hello world</s>
    <s xml:lang="es">Hola mundo</s>
  </linkGrp>
  <linkGrp>
    <s xml:lang="en">Goodbye</s>
    <s xml:lang="es">Adios</s>
  </linkGrp>
</cesAlign>"#;

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(xml_content.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel(100);

        let count =
            CorpusParser::parse_xml_alignment_file(tmp.path().to_str().unwrap(), "en", "es", &tx)
                .await
                .unwrap();
        drop(tx);

        assert_eq!(count, 2);
        let pairs = collect_translations(rx);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].source_text, "Hello world");
        assert_eq!(pairs[0].target_text, "Hola mundo");
        assert_eq!(pairs[1].source_text, "Goodbye");
        assert_eq!(pairs[1].target_text, "Adios");
    }

    #[tokio::test]
    async fn parse_xml_alignment_skips_incomplete_pairs() {
        let xml_content = r#"<?xml version="1.0"?>
<cesAlign>
  <linkGrp>
    <s xml:lang="en">Only source</s>
  </linkGrp>
  <linkGrp>
    <s xml:lang="en">Has both</s>
    <s xml:lang="es">Tiene ambos</s>
  </linkGrp>
</cesAlign>"#;

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(xml_content.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel(100);

        let count =
            CorpusParser::parse_xml_alignment_file(tmp.path().to_str().unwrap(), "en", "es", &tx)
                .await
                .unwrap();
        drop(tx);

        assert_eq!(count, 1);
        let pairs = collect_translations(rx);
        assert_eq!(pairs[0].source_text, "Has both");
        assert_eq!(pairs[0].target_text, "Tiene ambos");
    }

    proptest! {
        #[test]
        fn tmx_parsing_roundtrip(
            source in "[a-zA-Z0-9]{1,50}",
            target in "[a-zA-Z0-9]{1,50}",
        ) {
            let content = format!(
                r#"<?xml version="1.0"?><tmx><body><tu><tuv xml:lang="en"><seg>{source}</seg></tuv><tuv xml:lang="es"><seg>{target}</seg></tuv></tu></body></tmx>"#
            );
            let mut tmp = NamedTempFile::new().unwrap();
            tmp.write_all(content.as_bytes()).unwrap();
            let (tx, rx) = mpsc::channel(100);

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let count = rt.block_on(async {
                CorpusParser::parse_tmx_file(
                    tmp.path().to_str().unwrap(), "en", "es", &tx,
                ).await.unwrap()
            });
            drop(tx);

            prop_assert_eq!(count, 1);
            let pairs = collect_translations(rx);
            prop_assert_eq!(&pairs[0].source_text, &source);
            prop_assert_eq!(&pairs[0].target_text, &target);
        }

        #[test]
        fn xml_alignment_parsing_roundtrip(
            source in "[a-zA-Z0-9]{1,50}",
            target in "[a-zA-Z0-9]{1,50}",
        ) {
            let content = format!(
                r#"<?xml version="1.0"?><cesAlign><linkGrp><s xml:lang="en">{source}</s><s xml:lang="es">{target}</s></linkGrp></cesAlign>"#
            );
            let mut tmp = NamedTempFile::new().unwrap();
            tmp.write_all(content.as_bytes()).unwrap();
            let (tx, rx) = mpsc::channel(100);

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let count = rt.block_on(async {
                CorpusParser::parse_xml_alignment_file(
                    tmp.path().to_str().unwrap(), "en", "es", &tx,
                ).await.unwrap()
            });
            drop(tx);

            prop_assert_eq!(count, 1);
            let pairs = collect_translations(rx);
            prop_assert_eq!(&pairs[0].source_text, &source);
            prop_assert_eq!(&pairs[0].target_text, &target);
        }
    }
}
