//! Docker's build-context ignore rules, applied before any source leaves this host.
//!
//! Matching follows `moby/patternmatcher`: gitignore semantics differ, so no
//! gitignore library is used.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Component, Path, PathBuf},
};

use regex::Regex;

use super::Error;

/// Context entries the build may read, plus the ignore file that selected them.
pub(super) struct Selection {
    pub paths: BTreeSet<PathBuf>,
    pub ignore: Vec<u8>,
}

/// The recipe whose ignore rules select a build context.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Rules {
    /// A Dockerfile's own ignore file replaces the context's `.dockerignore`.
    Dockerfile(PathBuf),
    /// The config's `exclude` list follows the Docker patterns, and the config
    /// stays readable. `None` names the default `railpack.json`, which is
    /// optional however it is named.
    Railpack(Option<PathBuf>),
}

/// `<Dockerfile>.dockerignore`, which Docker prefers over the context's ignore file.
pub(super) fn own_ignore(dockerfile: &Path) -> PathBuf {
    let mut own = dockerfile.as_os_str().to_owned();
    own.push(".dockerignore");
    own.into()
}

/// Select the `context` entries a Build may read.
pub(super) fn select(context: &Path, rules: &Rules) -> Result<Selection, Error> {
    let (own, railpack) = match rules {
        Rules::Dockerfile(dockerfile) => (read_optional(&own_ignore(dockerfile))?, None),
        Rules::Railpack(config) => (None, Some(config.as_deref())),
    };
    // Paths kept even when a pattern excludes them.
    let mut controls = Vec::new();
    let ignore = match own {
        Some(own) => own,
        None => {
            // Docker keeps the active root ignore file, even if it excludes itself.
            controls.push(PathBuf::from(".dockerignore"));
            read_optional(&context.join(".dockerignore"))?.unwrap_or_default()
        }
    };
    let mut patterns = ignore_file_patterns(&ignore);
    if let Some(config) = railpack {
        let (excludes, kept) = railpack_rules(context, config)?;
        patterns.extend(excludes);
        controls.extend(kept);
    }
    let matcher = Matcher::new(&patterns)?;
    let mut paths = BTreeSet::new();
    walk(context, Path::new(""), &matcher, &controls, &mut paths)
        .map_err(|error| Error::Io(format!("select build context: {error}")))?;
    Ok(Selection { paths, ignore })
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::Io(format!("read {}: {error}", path.display()))),
    }
}

/// The config's exclusions and the paths preparation must still read.
/// Railpack 0.39.0 appends configured exclusions after Docker patterns, so
/// config negations can re-include Docker-ignored source.
fn railpack_rules(
    context: &Path,
    config: Option<&Path>,
) -> Result<(Vec<String>, Vec<PathBuf>), Error> {
    let invalid = |message: &str| Error::Invalid(message.into());
    let default = Path::new("railpack.json");
    let config = config.unwrap_or(default);
    if config.as_os_str().is_empty()
        || !config
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(invalid(
            "Railpack configuration must stay inside build.context",
        ));
    }
    let path = context.join(config);
    let mut kept = vec![PathBuf::from(clean(&config.to_string_lossy()))];
    match path.canonicalize() {
        Ok(resolved) => kept.push(
            resolved
                .strip_prefix(context)
                .map_err(|_| invalid("Railpack configuration escapes build.context"))?
                .to_owned(),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(invalid("cannot read Railpack configuration")),
    }
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        // Railpack's default config is optional, even when named explicitly.
        Err(error) if error.kind() == io::ErrorKind::NotFound && config == default => {
            return Ok((Vec::new(), kept));
        }
        Err(_) => return Err(invalid("cannot read Railpack configuration")),
    };
    #[derive(serde::Deserialize)]
    struct Configured {
        #[serde(default)]
        exclude: Vec<String>,
    }
    // Railpack accepts JSON with comments and trailing commas.
    let configured: Configured = standard_json(&content)
        .and_then(|json| serde_json::from_str(&json).ok())
        .ok_or_else(|| invalid("invalid Railpack configuration"))?;
    Ok((configured.exclude, kept))
}

fn walk(
    root: &Path,
    relative: &Path,
    matcher: &Matcher,
    controls: &[PathBuf],
    included: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
    let entries = match fs::read_dir(root.join(relative)) {
        Ok(entries) => entries,
        // BuildKit ignores permission errors in excluded directories, including
        // those visited to look for negated patterns.
        Err(error)
            if error.kind() == io::ErrorKind::PermissionDenied
                && !relative.as_os_str().is_empty()
                && matcher.matches_or_parent_matches(relative) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let mut names = entries
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    for name in names {
        let path = relative.join(name);
        // A kept path keeps its parents traversable; the frontend applies the same patterns.
        let ignored = matcher.matches_or_parent_matches(&path)
            && !controls.iter().any(|control| control.starts_with(&path));
        let directory = fs::symlink_metadata(root.join(&path))?.is_dir();
        if ignored {
            // Only descend when an exception can select a descendant (BuildKit semantics).
            if directory && matcher.exception_can_match_descendant(&path) {
                walk(root, &path, matcher, controls, included)?;
            }
            continue;
        }
        for parent in path
            .ancestors()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            included.insert(parent.to_owned());
        }
        if directory {
            walk(root, &path, matcher, controls, included)?;
        }
    }
    Ok(())
}

/// Docker's `.dockerignore` reader: comments before trimming, a leading BOM,
/// cleaned paths, and anchoring at the context root.
fn ignore_file_patterns(content: &[u8]) -> Vec<String> {
    let content = String::from_utf8_lossy(content);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    content
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let line = line.trim();
            let (invert, pattern) = match line.strip_prefix('!') {
                Some(pattern) => (true, pattern.trim()),
                None => (false, line),
            };
            if line.is_empty() {
                return None;
            }
            let mut pattern = if pattern.is_empty() {
                String::new()
            } else {
                clean(pattern)
            };
            if pattern.len() > 1 && pattern.starts_with('/') {
                pattern.remove(0);
            }
            Some(if invert {
                format!("!{pattern}")
            } else {
                pattern
            })
        })
        .collect()
}

struct Matcher {
    patterns: Vec<Pattern>,
}

struct Pattern {
    source: String,
    exclusion: bool,
    regex: Regex,
}

impl Matcher {
    fn new(patterns: &[String]) -> Result<Self, Error> {
        let patterns = patterns
            .iter()
            .map(|pattern| pattern.trim())
            .filter(|pattern| !pattern.is_empty())
            .map(|pattern| {
                let pattern = clean(pattern);
                let (exclusion, source) = match pattern.strip_prefix('!') {
                    Some("") => {
                        return Err(Error::Invalid("illegal exclusion pattern: \"!\"".into()));
                    }
                    Some(source) => (true, source.to_owned()),
                    None => (false, pattern),
                };
                let regex = Regex::new(&pattern_regex(&source)).map_err(|_| {
                    Error::Invalid(format!("invalid build context ignore pattern: {source}"))
                })?;
                Ok(Pattern {
                    source,
                    exclusion,
                    regex,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { patterns })
    }

    /// The last matching pattern decides, checked against the path and each parent.
    fn matches_or_parent_matches(&self, path: &Path) -> bool {
        let path = path.to_string_lossy();
        let parents: Vec<&str> = path.split('/').collect();
        let mut matched = false;
        for pattern in &self.patterns {
            if pattern.exclusion != matched {
                continue;
            }
            let found = pattern.regex.is_match(&path)
                || (1..parents.len()).any(|end| {
                    pattern
                        .regex
                        .is_match(&parents.get(..end).unwrap_or_default().join("/"))
                });
            if found {
                matched = !pattern.exclusion;
            }
        }
        matched
    }

    /// A wildcard cannot match outside the literal prefix preceding it. Remaining
    /// cases stay conservative so `**` and escaped patterns keep descendants.
    fn exception_can_match_descendant(&self, directory: &Path) -> bool {
        let directory = format!("{}/", directory.to_string_lossy());
        self.patterns
            .iter()
            .filter(|pattern| pattern.exclusion)
            .any(|pattern| {
                let prefix = pattern
                    .source
                    .find(['*', '[', ']', '?', '^', '\\'])
                    .map_or(pattern.source.as_str(), |wildcard| {
                        pattern.source.get(..wildcard).unwrap_or_default()
                    });
                if prefix.len() == pattern.source.len() {
                    prefix.starts_with(&directory)
                } else {
                    directory.starts_with(prefix) || prefix.starts_with(&directory)
                }
            })
    }
}

/// `patternmatcher`'s translation: `*` and `?` stop at `/`, `**` spans
/// directories, and `\` escapes the next character.
fn pattern_regex(pattern: &str) -> String {
    let mut regex = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                }
                regex.push_str(if chars.peek().is_none() {
                    ".*"
                } else {
                    "(.*/)?"
                });
            }
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '{' | '}' | '$' => {
                regex.push('\\');
                regex.push(ch);
            }
            '\\' => match chars.next() {
                Some(next) => regex.push_str(&regex::escape(&next.to_string())),
                None => regex.push_str("\\\\"),
            },
            _ => regex.push(ch),
        }
    }
    regex.push('$');
    regex
}

/// Go's `path.Clean`: the shortest lexically equivalent slash path.
fn clean(path: &str) -> String {
    let rooted = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            ".." if rooted => {}
            part => parts.push(part),
        }
    }
    let joined = parts.join("/");
    match (rooted, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".into(),
        (false, false) => joined,
    }
}

/// Strip comments and trailing commas, as `hujson.Standardize` does.
fn standard_json(text: &str) -> Option<String> {
    let mut json = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                json.push(ch);
                while let Some(ch) = chars.next() {
                    json.push(ch);
                    match ch {
                        '\\' => json.push(chars.next()?),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => while chars.next_if(|ch| *ch != '\n').is_some() {},
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = None;
                loop {
                    let ch = chars.next()?;
                    if previous == Some('*') && ch == '/' {
                        break;
                    }
                    previous = Some(ch);
                }
            }
            '}' | ']' => {
                let end = json.trim_end().len();
                if json.get(..end).is_some_and(|kept| kept.ends_with(',')) {
                    json.truncate(end - 1);
                }
                json.push(ch);
            }
            _ => json.push(ch),
        }
    }
    Some(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(patterns: &[&str]) -> Matcher {
        Matcher::new(&patterns.iter().map(ToString::to_string).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn patterns_follow_docker_matching() {
        for (patterns, path, expected) in [
            (&["*.log"][..], "debug.log", true),
            (&["*.log"], "logs/debug.log", false),
            (&["**/*.log"], "logs/deep/debug.log", true),
            (&["logs/**"], "logs/debug.log", true),
            (&["logs/**"], "logs", false),
            (&["cache"], "cache/nested/file", true),
            (&["cache", "!cache/keep"], "cache/keep", false),
            (&["cache", "!cache/keep"], "cache/drop", true),
            (&["cache", "!**/keep"], "cache/keep", false),
            (&["a?c"], "abc", true),
            (&["a?c"], "a/c", false),
            (&["[a-c]x"], "bx", true),
            (&["file.txt"], "fileXtxt", false),
            (&["\\*"], "*", true),
            (&["\\*"], "a", false),
        ] {
            assert_eq!(
                matcher(patterns).matches_or_parent_matches(Path::new(path)),
                expected,
                "{patterns:?} {path}"
            );
        }
        assert!(Matcher::new(&["[".into()]).is_err());
        assert!(Matcher::new(&["!".into()]).is_err());
    }

    #[test]
    fn ignore_files_are_read_like_docker() {
        assert_eq!(
            ignore_file_patterns(
                "\u{feff}# comment\n /cache/ \n!**/keep\n\n ./a/../b\n".as_bytes()
            ),
            ["cache", "!**/keep", "b"]
        );
        assert_eq!(clean("a//b/./c/.."), "a/b");
        assert_eq!(clean("../a"), "../a");
        assert_eq!(clean("/../a/"), "/a");
    }

    #[test]
    fn only_exceptions_below_an_ignored_directory_keep_it_walked() {
        let keep = matcher(&["cache", "!cache/keep"]);
        assert!(keep.exception_can_match_descendant(Path::new("cache")));
        assert!(!keep.exception_can_match_descendant(Path::new("other")));
        let wildcard = matcher(&["cache", "!**/keep"]);
        assert!(wildcard.exception_can_match_descendant(Path::new("other")));
    }

    #[test]
    fn railpack_config_accepts_comments_and_trailing_commas() {
        let json = standard_json(
            "{\n  // comment\n  \"exclude\": [\"a//b\", \"/*c*/\",], /* block */\n}\n",
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value, serde_json::json!({"exclude": ["a//b", "/*c*/"]}));
        assert!(standard_json("{\"a\": 1 /* open").is_none());
    }
}
