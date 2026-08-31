use super::*;

pub(in crate::workspace) fn content_with_requested_tags(
    note: &Note,
    old_content: Option<&str>,
) -> Result<String, String> {
    let requested_tags = normalize_tags(&note.tags);
    if parse_frontmatter_tags(&note.content) == requested_tags {
        return Ok(note.content.clone());
    }

    let action = if old_content.is_some() {
        "update"
    } else {
        "write"
    };
    update_frontmatter_tags_conservatively(&note.content, &requested_tags).map_err(|error| {
        format!(
            concat!(
                "Could not {} tags for {:?}: {} ",
                "Edit the tags in Markdown source instead. ",
                "If the frontmatter is hidden, reveal it from the note toolbar."
            ),
            action, note.title, error,
        )
    })
}

pub(in crate::workspace) fn update_frontmatter_tags_conservatively(
    content: &str,
    normalized_tags: &[String],
) -> Result<String, String> {
    let Some((body_start, body_end, line_ending)) = frontmatter_bounds(content) else {
        if normalized_tags.is_empty() {
            return Ok(content.to_owned());
        }
        if content
            .strip_prefix('\u{feff}')
            .unwrap_or(content)
            .lines()
            .next()
            .is_some_and(|line| line.trim() == "---")
        {
            return Err("the existing frontmatter is not closed".to_owned());
        }
        let (bom, body) = content
            .strip_prefix('\u{feff}')
            .map_or(("", content), |body| ("\u{feff}", body));
        let line_ending = if body.contains("\r\n") { "\r\n" } else { "\n" };
        let mut output = String::from(bom);
        output.push_str("---");
        output.push_str(line_ending);
        append_tag_block(&mut output, normalized_tags, line_ending);
        output.push_str("---");
        output.push_str(line_ending);
        output.push_str(line_ending);
        output.push_str(body);

        return Ok(output);
    };

    let body = &content[body_start..body_end];
    let tag_span = find_conservative_tag_span(body)?;
    let mut new_body = String::new();
    match tag_span {
        Some((start, end)) => {
            new_body.push_str(&body[..start]);
            if !normalized_tags.is_empty() {
                append_tag_block(&mut new_body, normalized_tags, line_ending);
            }
            new_body.push_str(&body[end..]);
        }
        None => {
            new_body.push_str(body);
            if !normalized_tags.is_empty() {
                if !new_body.is_empty() && !new_body.ends_with('\n') {
                    new_body.push_str(line_ending);
                }
                append_tag_block(&mut new_body, normalized_tags, line_ending);
            }
        }
    }
    Ok(format!(
        "{}{}{}",
        &content[..body_start],
        new_body,
        &content[body_end..]
    ))
}

pub(in crate::workspace) fn find_conservative_tag_span(
    body: &str,
) -> Result<Option<(usize, usize)>, String> {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let mut spans = Vec::new();
    let mut offset = 0;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let without_ending = trim_line_ending(line);
        let indented = without_ending.starts_with(' ') || without_ending.starts_with('\t');
        let Some((key, value)) = without_ending.split_once(':') else {
            offset += line.len();
            index += 1;
            continue;
        };
        if indented || !key.trim().eq_ignore_ascii_case("tags") {
            offset += line.len();
            index += 1;
            continue;
        }
        if value.contains('#')
            || matches!(
                value.trim().chars().next(),
                Some('&' | '*' | '!' | '|' | '>' | '{')
            )
        {
            return Err("the tags field uses comments, anchors, or complex YAML".to_owned());
        }
        let block_list = value.trim().is_empty();
        let start = offset;
        offset += line.len();
        index += 1;
        while block_list && index < lines.len() {
            let continuation = trim_line_ending(lines[index]);
            let trimmed = continuation.trim();
            let is_indented = continuation.starts_with(' ') || continuation.starts_with('\t');
            let is_unindented_list = !is_indented
                && (trimmed == "-" || trimmed.starts_with("- ") || trimmed.starts_with("-\t"));
            let is_list = is_unindented_list
                || (is_indented
                    && (trimmed == "-" || trimmed.starts_with("- ") || trimmed.starts_with("-\t")));
            if trimmed.is_empty() {
                offset += lines[index].len();
                index += 1;
                continue;
            }
            if is_list {
                if trimmed.starts_with('#') || trimmed.contains(" #") {
                    return Err("the tags field contains comments".to_owned());
                }
                let scalar = trimmed.trim_start_matches('-').trim();
                if matches!(
                    scalar.chars().next(),
                    Some('&' | '*' | '!' | '|' | '>' | '{' | '[')
                ) || (!scalar.starts_with(['\'', '"']) && scalar.contains(": "))
                {
                    return Err("the tags field uses complex YAML".to_owned());
                }
                offset += lines[index].len();
                index += 1;
                continue;
            }
            if is_indented {
                return Err("the tags field uses complex YAML".to_owned());
            }
            break;
        }
        spans.push((start, offset));
    }
    if spans.len() > 1 {
        return Err("frontmatter contains more than one top-level tags field".to_owned());
    }
    Ok(spans.into_iter().next())
}

pub(in crate::workspace) fn append_tag_block(
    output: &mut String,
    tags: &[String],
    line_ending: &str,
) {
    output.push_str("tags:");
    output.push_str(line_ending);
    for tag in tags {
        output.push_str("  - \"");
        output.push_str(&escape_yaml_double_quoted(tag));
        output.push('"');
        output.push_str(line_ending);
    }
}

pub(in crate::workspace) fn parse_frontmatter_tags(content: &str) -> Vec<String> {
    let Some((body_start, body_end, _)) = frontmatter_bounds(content) else {
        return Vec::new();
    };
    let body = &content[body_start..body_end];
    let mut tags = Vec::new();
    let mut reading_tag_list = false;
    for raw_line in body.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = raw_line.starts_with(' ') || raw_line.starts_with('\t');
        if reading_tag_list
            && (indented
                || trimmed == "-"
                || trimmed.starts_with("- ")
                || trimmed.starts_with("-\t"))
        {
            if let Some(value) = trimmed.strip_prefix('-') {
                push_tag(&mut tags, parse_yaml_scalar(value.trim()));
            }
            continue;
        }
        reading_tag_list = false;
        if indented {
            continue;
        }
        let Some((key, value)) = raw_line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("tags") {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            reading_tag_list = true;
        } else if value.starts_with('[') {
            for value in parse_inline_yaml_list(value) {
                push_tag(&mut tags, value);
            }
        } else {
            push_tag(&mut tags, parse_yaml_scalar(value));
        }
    }
    tags
}

pub(in crate::workspace) fn frontmatter_bounds(content: &str) -> Option<(usize, usize, &str)> {
    let bom_length = if content.starts_with('\u{feff}') {
        3
    } else {
        0
    };
    let remaining = &content[bom_length..];
    let first_end = remaining
        .find('\n')
        .map(|index| index + 1)
        .unwrap_or(remaining.len());
    let first = &remaining[..first_end];
    if trim_line_ending(first).trim() != "---" {
        return None;
    }
    let line_ending = if first.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let body_start = bom_length + first_end;
    let mut cursor = body_start;
    for line in content[body_start..].split_inclusive('\n') {
        let trimmed = trim_line_ending(line).trim();
        if trimmed == "---" || trimmed == "..." {
            return Some((body_start, cursor, line_ending));
        }
        cursor += line.len();
    }
    None
}

pub(in crate::workspace) fn parse_inline_yaml_list(value: &str) -> Vec<String> {
    let Some(end) = value.rfind(']') else {
        return vec![parse_yaml_scalar(value)];
    };
    let inner = &value[1..end];
    let mut values = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
        } else if character == '\'' || character == '"' {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == ',' && quote.is_none() {
            values.push(parse_yaml_scalar(inner[start..index].trim()));
            start = index + 1;
        }
    }
    values.push(parse_yaml_scalar(inner[start..].trim()));
    values
}

pub(in crate::workspace) fn parse_yaml_scalar(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let mut output = String::new();
        let mut escaped = false;
        for character in value[1..value.len() - 1].chars() {
            if escaped {
                output.push(match character {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                output.push(character);
            }
        }

        return output;
    }
    value
        .find(" #")
        .map(|index| &value[..index])
        .unwrap_or(value)
        .trim()
        .to_owned()
}

pub(in crate::workspace) fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for tag in tags {
        push_tag(&mut result, tag.clone());
    }
    result
}

pub(in crate::workspace) fn push_tag(tags: &mut Vec<String>, tag: String) {
    let tag = tag.trim().trim_start_matches('#').trim();
    if !tag.is_empty() && !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_owned());
    }
}

pub(in crate::workspace) fn escape_yaml_double_quoted(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other if other.is_control() => output.push(' '),
            other => output.push(other),
        }
    }
    output
}
