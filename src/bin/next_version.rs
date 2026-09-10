//! Prints the next version tag implied by the Conventional Commit messages
//! landed since the most recent version tag, or nothing at all when nothing
//! release-worthy has landed. Empty output is how the workflow decides not to
//! release, so it must stay empty rather than becoming "none" or "0.0.0".
//!
//! This is a CI helper, not part of grove. It lives behind the `ci` feature so
//! that `cargo install --git` skips it and nobody ends up with a `next-version`
//! binary they did not ask for.
//!
//! The rules are Conventional Commits as clarity applies them: `feat` → minor,
//! `fix` → patch, a `!` marker or a `BREAKING CHANGE:` footer → major whatever
//! the type says, and everything else → no release. Plain semver throughout,
//! with no 0.x special case: a breaking change at 0.4.1 is 1.0.0.

use std::process::Command;

/// A semantic version bump. Ordered, so the highest across a range of commits
/// is just a `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Bump {
    None,
    Patch,
    Minor,
    Major,
}

fn main() {
    let latest = match latest_tag() {
        Ok(tag) => tag,
        Err(why) => fail(&why),
    };
    let messages = match commit_messages_since(latest.as_deref()) {
        Ok(messages) => messages,
        Err(why) => fail(&why),
    };
    match next(latest.as_deref(), bump_for(&messages)) {
        Ok(Some(version)) => println!("{version}"),
        // Nothing at all, not even a newline: the workflow gates on empty.
        Ok(None) => {}
        Err(why) => fail(&why),
    }
}

fn fail(why: &str) -> ! {
    eprintln!("next-version: {why}");
    std::process::exit(1);
}

/// The highest version tag in the repository, or `None` when there is not one
/// yet. `--sort=-v:refname` orders numerically, so 0.10.0 beats 0.9.0.
fn latest_tag() -> Result<Option<String>, String> {
    let out = Command::new("git")
        .args(["tag", "--sort=-v:refname"])
        .output()
        .map_err(|e| format!("git tag: {e}"))?;
    if !out.status.success() {
        return Err(format!("git tag: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|line| parse_version(line).is_some())
        .map(str::to_owned))
}

/// Every commit message reachable from HEAD but not from `latest` — the whole
/// history when there is no tag yet, which is what makes a first release work.
///
/// NUL-separated, because a message body spans lines and splitting on newlines
/// would read each paragraph as its own commit.
fn commit_messages_since(latest: Option<&str>) -> Result<Vec<String>, String> {
    let mut args = vec!["log", "-z", "--format=%B"];
    let range;
    if let Some(tag) = latest {
        range = format!("{tag}..HEAD");
        args.push(&range);
    }
    let out = Command::new("git")
        .args(&args)
        .output()
        .map_err(|e| format!("git log: {e}"))?;
    if !out.status.success() {
        return Err(format!("git log: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|m| !m.trim().is_empty())
        .map(str::to_owned)
        .collect())
}

/// The highest bump any of these commits asks for.
fn bump_for(messages: &[String]) -> Bump {
    messages
        .iter()
        .map(|m| bump_for_one(m))
        .max()
        .unwrap_or(Bump::None)
}

fn bump_for_one(message: &str) -> Bump {
    let message = message.trim();
    let subject = message.lines().next().unwrap_or_default();
    let Some((kind, bang)) = parse_subject(subject) else {
        return Bump::None; // not a Conventional Commit, so it says nothing
    };
    if bang || has_breaking_footer(message) {
        return Bump::Major;
    }
    match kind.as_str() {
        "feat" => Bump::Minor,
        "fix" => Bump::Patch,
        _ => Bump::None,
    }
}

/// Split `type(scope)!:` into its type and whether the breaking marker is
/// there. `None` for anything that is not shaped like a Conventional Commit.
fn parse_subject(subject: &str) -> Option<(String, bool)> {
    let head = &subject[..subject.find(':')?];
    let (head, bang) = match head.strip_suffix('!') {
        Some(rest) => (rest, true),
        None => (head, false),
    };
    // A scope is optional, but a half-open one means this is prose with a
    // colon in it rather than a commit type.
    let kind = match head.find('(') {
        Some(open) if head.ends_with(')') => &head[..open],
        Some(_) => return None,
        None => head,
    };
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((kind.to_ascii_lowercase(), bang))
}

/// A breaking footer has to start its own line. Mentioning a breaking change
/// mid-sentence in a body is a description, not a declaration.
fn has_breaking_footer(message: &str) -> bool {
    message
        .lines()
        .any(|line| line.starts_with("BREAKING CHANGE:") || line.starts_with("BREAKING-CHANGE:"))
}

/// The tag to cut, or `None` when the bump is `None`. Unprefixed, matching the
/// 0.1.0 tag grove already published.
fn next(latest: Option<&str>, bump: Bump) -> Result<Option<String>, String> {
    if bump == Bump::None {
        return Ok(None);
    }
    let (mut major, mut minor, mut patch) = match latest {
        // No tag yet: count up from 0.0.0, so a first feat release is 0.1.0.
        None => (0, 0, 0),
        Some(tag) => {
            parse_version(tag).ok_or_else(|| format!("latest tag {tag:?} is not X.Y.Z"))?
        }
    };
    match bump {
        Bump::Major => (major, minor, patch) = (major + 1, 0, 0),
        Bump::Minor => (minor, patch) = (minor + 1, 0),
        Bump::Patch => patch += 1,
        Bump::None => unreachable!("returned above"),
    }
    Ok(Some(format!("{major}.{minor}.{patch}")))
}

/// `X.Y.Z` and nothing else — no `v`, no pre-release suffix, so a tag like
/// `0.2.0-rc1` is ignored rather than counted as the latest release.
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.split('.');
    let out = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bump(messages: &[&str]) -> Bump {
        bump_for(
            &messages
                .iter()
                .map(|m| (*m).to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn feat_is_minor_and_fix_is_patch() {
        assert_eq!(bump(&["feat: add a thing"]), Bump::Minor);
        assert_eq!(bump(&["fix: unbreak a thing"]), Bump::Patch);
        assert_eq!(bump(&["feat(tree): scoped still counts"]), Bump::Minor);
    }

    #[test]
    fn everything_else_releases_nothing() {
        for message in [
            "docs: explain the thing",
            "ci: bump an action",
            "style: gofmt equivalent",
            "refactor: move a thing",
            "chore: tidy up",
            "test: cover a case",
            // The one that started this: prose is not a Conventional Commit.
            "Follow the disk instead of waiting to be told",
            // Prose that merely contains a colon must not parse as a type.
            "Screenshots: keep herdr's frame in shot",
        ] {
            assert_eq!(bump(&[message]), Bump::None, "{message:?}");
        }
    }

    #[test]
    fn a_colon_in_prose_is_not_a_commit_type() {
        // Prose often has a colon in it. Only a single alphabetic word, with an
        // optional closed scope, counts as a type.
        assert_eq!(parse_subject("Fix the thing: really"), None);
        assert_eq!(parse_subject("feat something: no colon after type"), None);
        assert_eq!(parse_subject("feat(unclosed: scope"), None);
        assert_eq!(parse_subject("no colon at all"), None);
        assert_eq!(parse_subject("feat: a"), Some(("feat".into(), false)));
        assert_eq!(
            parse_subject("FEAT: a"),
            Some(("feat".into(), false)),
            "case-insensitive"
        );
        assert_eq!(parse_subject("fix(api)!: a"), Some(("fix".into(), true)));
    }

    #[test]
    fn a_bang_or_a_breaking_footer_is_major_whatever_the_type() {
        assert_eq!(bump(&["feat!: drop the old format"]), Bump::Major);
        // `refactor` alone releases nothing; the bang overrides that.
        assert_eq!(bump(&["refactor(api)!: rename a thing"]), Bump::Major);
        assert_eq!(
            bump(&["feat: rework events\n\nBREAKING CHANGE: layout changed"]),
            Bump::Major
        );
        assert_eq!(bump(&["fix: x\n\nBREAKING-CHANGE: y"]), Bump::Major);
    }

    #[test]
    fn a_breaking_change_mentioned_mid_sentence_is_not_a_footer() {
        assert_eq!(
            bump(&["fix: avoid a BREAKING CHANGE: in the body text"]),
            Bump::Patch,
            "the footer has to start its own line"
        );
    }

    #[test]
    fn the_highest_bump_across_the_range_wins() {
        assert_eq!(bump(&["fix: a", "feat: b"]), Bump::Minor);
        assert_eq!(bump(&["feat: a", "fix!: b"]), Bump::Major);
        assert_eq!(bump(&["docs: a", "fix: b"]), Bump::Patch);
        assert_eq!(bump(&["docs: a", "chore: b"]), Bump::None);
        assert_eq!(bump(&[]), Bump::None);
    }

    #[test]
    fn versions_count_up_from_the_latest_tag() {
        assert_eq!(
            next(Some("0.1.0"), Bump::Minor).unwrap().as_deref(),
            Some("0.2.0")
        );
        assert_eq!(
            next(Some("0.1.2"), Bump::Patch).unwrap().as_deref(),
            Some("0.1.3")
        );
        // No 0.x special case: breaking means 1.0.0, as it does in clarity.
        assert_eq!(
            next(Some("0.4.1"), Bump::Major).unwrap().as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            next(Some("1.2.3"), Bump::Major).unwrap().as_deref(),
            Some("2.0.0")
        );
        // A minor bump resets the patch, a major resets both.
        assert_eq!(
            next(Some("1.2.3"), Bump::Minor).unwrap().as_deref(),
            Some("1.3.0")
        );
    }

    #[test]
    fn no_tag_yet_counts_up_from_zero() {
        assert_eq!(next(None, Bump::Patch).unwrap().as_deref(), Some("0.0.1"));
        assert_eq!(next(None, Bump::Minor).unwrap().as_deref(), Some("0.1.0"));
        assert_eq!(next(None, Bump::Major).unwrap().as_deref(), Some("1.0.0"));
    }

    #[test]
    fn nothing_release_worthy_produces_no_tag() {
        assert_eq!(next(Some("0.1.0"), Bump::None).unwrap(), None);
        assert_eq!(next(None, Bump::None).unwrap(), None);
    }

    #[test]
    fn only_bare_x_y_z_tags_count_as_versions() {
        assert_eq!(parse_version("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_version("10.20.30"), Some((10, 20, 30)));
        // A `v` prefix, a pre-release or a stray fourth part is not our scheme,
        // and must not be picked up as "the latest release".
        assert_eq!(parse_version("v0.1.0"), None);
        assert_eq!(parse_version("0.2.0-rc1"), None);
        assert_eq!(parse_version("0.1.0.1"), None);
        assert_eq!(parse_version("0.1"), None);
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn an_unparseable_latest_tag_is_an_error_not_a_guess() {
        assert!(next(Some("nightly"), Bump::Patch).is_err());
    }
}
