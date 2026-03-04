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

    async fn send_pair(
        source: &mut Option<String>,
        target: &mut Option<String>,
        sender: &Sender<TranslationPair>,
        count: &mut usize,
    ) -> bool {
        if let (Some(src), Some(tgt)) = (source.take(), target.take()) {
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
