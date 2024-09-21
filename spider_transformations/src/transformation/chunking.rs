use regex::Regex;
use spider::lazy_static::lazy_static;
use spider::serde::de::{self, MapAccess, Visitor};
use spider::serde::ser::SerializeStruct;
use spider::serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

lazy_static! {
    /// regex for sentence transformation.
    static ref BY_SENTENCE: Regex = Regex::new(r"[.!?]\s+").unwrap();
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ChunkingAlgorithm {
    #[default]
    /// None
    No,
    /// Chunk by words, taking a specified number of words per chunk
    ByWords(usize),
    /// Chunk by lines, taking a specified number of lines per chunk
    ByLines(usize),
    /// Chunk by character length, taking a specified number of characters per chunk
    ByCharacterLength(usize),
    /// Chunk by sentences, taking a specified number of sentences per chunk
    BySentence(usize),
}

// Custom `Serialize` implementation
impl Serialize for ChunkingAlgorithm {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            ChunkingAlgorithm::No => serializer.serialize_str("none"),
            ChunkingAlgorithm::ByWords(value) => {
                let mut state = serializer.serialize_struct("ChunkingAlgorithm", 2)?;
                state.serialize_field("type", "bywords")?;
                state.serialize_field("value", &value)?;
                state.end()
            }
            ChunkingAlgorithm::ByLines(value) => {
                let mut state = serializer.serialize_struct("ChunkingAlgorithm", 2)?;
                state.serialize_field("type", "bylines")?;
                state.serialize_field("value", &value)?;
                state.end()
            }
            ChunkingAlgorithm::ByCharacterLength(value) => {
                let mut state = serializer.serialize_struct("ChunkingAlgorithm", 2)?;
                state.serialize_field("type", "bycharacterlength")?;
                state.serialize_field("value", &value)?;
                state.end()
            }
            ChunkingAlgorithm::BySentence(value) => {
                let mut state = serializer.serialize_struct("ChunkingAlgorithm", 2)?;
                state.serialize_field("type", "bysentence")?;
                state.serialize_field("value", &value)?;
                state.end()
            }
        }
    }
}

// Custom `Deserialize` implementation
impl<'de> Deserialize<'de> for ChunkingAlgorithm {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Type,
            Value,
        }

        struct ChunkingAlgorithmVisitor;

        impl<'de> Visitor<'de> for ChunkingAlgorithmVisitor {
            type Value = ChunkingAlgorithm;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct ChunkingAlgorithm")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "none" => Ok(ChunkingAlgorithm::No),
                    _ => Err(de::Error::unknown_variant(
                        value,
                        &[
                            "none",
                            "bywords",
                            "bylines",
                            "bycharacterlength",
                            "bysentence",
                        ],
                    )),
                }
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut type_str = None;
                let mut value = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Type => {
                            if type_str.is_some() {
                                return Err(de::Error::duplicate_field("type"));
                            }
                            type_str = Some(map.next_value()?);
                        }
                        Field::Value => {
                            if value.is_some() {
                                return Err(de::Error::duplicate_field("value"));
                            }
                            value = Some(map.next_value()?);
                        }
                    }
                }
                let type_str: String = type_str.ok_or_else(|| de::Error::missing_field("type"))?;
                match type_str.as_str() {
                    "none" => Ok(ChunkingAlgorithm::No),
                    "bywords" => Ok(ChunkingAlgorithm::ByWords(
                        value.ok_or_else(|| de::Error::missing_field("value"))?,
                    )),
                    "bylines" => Ok(ChunkingAlgorithm::ByLines(
                        value.ok_or_else(|| de::Error::missing_field("value"))?,
                    )),
                    "bycharacterlength" => Ok(ChunkingAlgorithm::ByCharacterLength(
                        value.ok_or_else(|| de::Error::missing_field("value"))?,
                    )),
                    "bysentence" => Ok(ChunkingAlgorithm::BySentence(
                        value.ok_or_else(|| de::Error::missing_field("value"))?,
                    )),
                    _ => Err(de::Error::unknown_variant(
                        &type_str,
                        &[
                            "none",
                            "bywords",
                            "bylines",
                            "bycharacterlength",
                            "bysentence",
                        ],
                    )),
                }
            }
        }

        const FIELDS: &[&str] = &["type", "value"];
        deserializer.deserialize_struct("ChunkingAlgorithm", FIELDS, ChunkingAlgorithmVisitor)
    }
}

/// chunk the text output
pub fn chunk_text(text: &str, algorithm: ChunkingAlgorithm) -> Vec<String> {
    match algorithm {
        ChunkingAlgorithm::ByWords(words_per_chunk) => chunk_by_words(text, words_per_chunk),
        ChunkingAlgorithm::ByLines(lines_per_chunk) => chunk_by_lines(text, lines_per_chunk),
        ChunkingAlgorithm::ByCharacterLength(char_length) => {
            chunk_by_character_length(text, char_length)
        }
        ChunkingAlgorithm::BySentence(sentences_per_chunk) => {
            chunk_by_sentence(text, sentences_per_chunk)
        }
        ChunkingAlgorithm::No => Default::default(),
    }
}

/// Chunk by sentence.
fn chunk_by_sentence(text: &str, sentences_per_chunk: usize) -> Vec<String> {
    let sentences: Vec<&str> = BY_SENTENCE.split(text).collect();
    // Group sentences into chunks
    let mut chunks = Vec::new();
    for chunk in sentences.chunks(sentences_per_chunk) {
        chunks.push(chunk.join(" ").trim().to_string());
    }
    chunks
}

/// chunk by words
fn chunk_by_words(text: &str, words_per_chunk: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_chunk = vec![];

    for word in text.split_whitespace() {
        current_chunk.push(word);
        if current_chunk.len() >= words_per_chunk {
            result.push(current_chunk.join(" "));
            current_chunk.clear();
        }
    }

    if !current_chunk.is_empty() {
        result.push(current_chunk.join(" "));
    }

    result
}

/// chunk by lines.
fn chunk_by_lines(text: &str, lines_per_chunk: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_chunk = vec![];
    for line in text.lines() {
        current_chunk.push(line);
        if current_chunk.len() >= lines_per_chunk {
            result.push(current_chunk.join("\n"));
            current_chunk.clear();
        }
    }
    if !current_chunk.is_empty() {
        result.push(current_chunk.join("\n"));
    }
    result
}

/// chunk by char length.
fn chunk_by_character_length(text: &str, char_length: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_chunk = String::new();
    for c in text.chars() {
        current_chunk.push(c);
        if current_chunk.len() >= char_length {
            result.push(current_chunk.clone());
            current_chunk.clear();
        }
    }
    if !current_chunk.is_empty() {
        result.push(current_chunk);
    }
    result
}
