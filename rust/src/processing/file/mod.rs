pub struct Frequency {}

pub trait FrequencyFileParser {
    pub fn parse_file(file: LanguageFileInfo) -> Vec<Frequency>;
}

pub struct Translation {}

pub trait TranslationFileParser {
    pub fn parse_file(file: LanguageFileInfo) -> Vec<Translation>;
}
