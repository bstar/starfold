//! Bounded IMDb title discovery on the preview worker, with an offline cache.
use super::super::model::Document;
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, sync::Arc, time::Duration};
#[derive(Clone, Deserialize, Serialize)]
struct Title {
    id: String,
    l: String,
    y: Option<u16>,
    s: Option<String>,
    qid: Option<String>,
    i: Option<Poster>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Poster {
    #[serde(rename = "imageUrl")]
    url: String,
}
#[derive(Deserialize)]
struct Suggestions {
    #[serde(default)]
    d: Vec<Title>,
}
fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn query(text: &str) -> (String, Option<u16>) {
    let spaced = text.replace(['.', '_', '(', ')', '[', ']', '{', '}'], " ");
    let words: Vec<_> = spaced.split_whitespace().collect();
    let year = words.iter().enumerate().find_map(|(i, word)| {
        word.parse::<u16>()
            .ok()
            .filter(|y| (1900..=2099).contains(y))
            .map(|y| (i, y))
    });
    let end = year.map(|(i, _)| i).unwrap_or_else(|| {
        words
            .iter()
            .position(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "br" | "bluray"
                        | "blu-ray"
                        | "remux"
                        | "web-dl"
                        | "webrip"
                        | "dvdrip"
                        | "720p"
                        | "1080p"
                        | "2160p"
                        | "4k"
                        | "x264"
                        | "x265"
                )
            })
            .unwrap_or(words.len())
    });
    (words[..end].join(" "), year.map(|(_, y)| y))
}
fn choose(titles: Vec<Title>, name: &str, year: Option<u16>, id: Option<&str>) -> Option<Title> {
    let mut matches = titles.into_iter().filter(|title| {
        if let Some(id) = id {
            return title.id == id;
        }
        matches!(title.qid.as_deref(), Some("movie" | "tvMovie"))
            && normalized(&title.l) == normalized(name)
            && year.is_none_or(|year| title.y == Some(year))
    });
    let result = matches.next()?;
    matches.next().is_none().then_some(result)
}
fn fetch(agent: &starkit::ureq::Agent, url: &str, limit: usize) -> anyhow::Result<Vec<u8>> {
    let mut response = agent.get(url).call()?;
    anyhow::ensure!(
        response.status().is_success(),
        "IMDb HTTP {}",
        response.status()
    );
    let mut bytes = vec![];
    response
        .body_mut()
        .as_reader()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= limit, "IMDb response exceeds limit");
    Ok(bytes)
}
pub(super) fn enrich(document: &mut Document, path: &Path, id: Option<&str>) {
    if std::env::var_os("STARFOLD_MOVIE_LOOKUP").is_some_and(|v| v == "off") {
        return;
    }
    let name = document
        .fields
        .iter()
        .find(|f| f.label.eq_ignore_ascii_case("title"))
        .map(|f| f.value.as_str())
        .unwrap_or("");
    let (name, mut year) = query(name);
    if year.is_none() {
        year = query(&path.file_stem().unwrap_or_default().to_string_lossy()).1;
    }
    if (name.is_empty() || name.len() > 160) && id.is_none() {
        return;
    }
    let search = id.unwrap_or(&name);
    let key = format!("{}-{}", normalized(search), year.unwrap_or(0));
    let root = crate::paths::PATHS
        .cache_dir()
        .ok()
        .map(|p| p.join("movies"));
    let file = root.as_ref().map(|p| p.join(format!("{key}.json")));
    let cached = file
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .filter(|bytes| bytes.len() <= 64 * 1024)
        .and_then(|bytes| serde_json::from_slice::<Title>(&bytes).ok());
    let agent: starkit::ureq::Agent =
        starkit::net::builder(concat!("starfold/", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(3)))
            .build()
            .into();
    let title = cached.or_else(|| {
        let encoded: String = search
            .as_bytes()
            .iter()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() {
                    char::from(*byte).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect();
        let url = format!(
            "https://v3.sg.media-imdb.com/suggestion/{}/{encoded}.json",
            search.chars().next().unwrap_or('t').to_ascii_lowercase()
        );
        let result = fetch(&agent, &url, 64 * 1024)
            .and_then(|bytes| Ok(serde_json::from_slice::<Suggestions>(&bytes)?));
        match result {
            Ok(suggestions) => choose(suggestions.d, &name, year, id),
            Err(error) => {
                tracing::debug!(%error, "IMDb lookup unavailable; using local movie tags");
                None
            }
        }
    });
    let Some(title) = title else {
        return;
    };
    if super::imdb_id(&title.id).as_deref() != Some(title.id.as_str()) {
        return;
    }
    if let Some(file) = file {
        if let Some(root) = &root {
            let _ = std::fs::create_dir_all(root);
        }
        if let Ok(bytes) = serde_json::to_vec(&title) {
            let _ = std::fs::write(file, bytes);
        }
    }
    document.fields.retain(|f| {
        !matches!(
            f.label.to_ascii_lowercase().as_str(),
            "title" | "year" | "imdb" | "cast"
        )
    });
    document.field("Title", &title.l);
    if let Some(year) = title.y {
        document.field("Year", year);
    }
    if let Some(cast) = &title.s {
        document.field("Cast", cast);
    }
    document.field("IMDb", format!("https://www.imdb.com/title/{}/", title.id));
    let Some(poster) = title
        .i
        .filter(|p| p.url.starts_with("https://m.media-amazon.com/images/"))
    else {
        return;
    };
    let image_file = root.map(|p| p.join(format!("{}.jpg", title.id)));
    let bytes = image_file
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .filter(|b| b.len() <= 2 * 1024 * 1024)
        .or_else(|| {
            fetch(
                &agent,
                &poster.url.replace("._V1_.", "._V1_UX400_."),
                2 * 1024 * 1024,
            )
            .ok()
        });
    if let Some(bytes) = bytes {
        if let Ok(mut reader) =
            starkit::image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()
        {
            let mut limits = starkit::image::Limits::default();
            limits.max_image_width = Some(2048);
            limits.max_image_height = Some(2048);
            limits.max_alloc = Some(16 * 1024 * 1024);
            reader.limits(limits);
            if let Ok(image) = reader.decode() {
                document.image = Some(Arc::new(image.to_rgba8()));
                if let Some(file) = image_file {
                    let _ = std::fs::write(file, bytes);
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_names_match_the_right_movie_without_guessing_remakes() {
        assert_eq!(
            query("True Grit 2010 BR 720p x264 Qmax"),
            ("True Grit".into(), Some(2010))
        );
        assert_eq!(
            query("The.Hobbit.2013.1080p.BluRay.mkv"),
            ("The Hobbit".into(), Some(2013))
        );
        let data = r#"{"d":[{"id":"tt1403865","l":"True Grit","y":2010,"qid":"movie"},{"id":"tt0065126","l":"True Grit","y":1969,"qid":"movie"}]}"#;
        let parsed = || serde_json::from_str::<Suggestions>(data).unwrap().d;
        assert_eq!(
            choose(parsed(), "True Grit", Some(2010), None).unwrap().id,
            "tt1403865"
        );
        assert!(choose(parsed(), "True Grit", None, None).is_none());
        assert!(choose(parsed(), "True Grit", Some(2000), None).is_none());
    }
}
