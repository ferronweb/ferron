use std::path::Path;

fn kdlite_error_near(pos: usize, file_contents: &str) -> String {
    let part = file_contents
        .split_at_checked(pos)
        .map(|split| {
            split
                .1
                .split_at_checked(50)
                .map_or(split.1, |split2| split2.0)
        })
        .and_then(|part| if part.is_empty() { None } else { Some(part) });
    part.map_or("<end or out of bounds>".to_string(), |p| {
        snailquote::escape(p).to_string()
    })
}

fn display_kdlite_error(err: &kdlite::stream::Error, file_contents: &str) -> String {
    match err {
        kdlite::stream::Error::ExpectedSpace(index) => {
            format!(
                "Expected space near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::ExpectedCloseParen(index) => {
            format!(
                "Expected `)` near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::ExpectedComment(index) => format!(
            "Expected single-line comment near {}",
            kdlite_error_near(*index, file_contents)
        ),
        kdlite::stream::Error::ExpectedNewline(index) => {
            format!(
                "Expected newline near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::ExpectedString(index) => {
            format!(
                "Expected string near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::ExpectedValue(index) => {
            format!(
                "Expected value near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::UnexpectedCloseBracket(index) => {
            format!(
                "Unexpected `}}` near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::UnexpectedNewline(index) => {
            format!(
                "Unexpected newline near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::InvalidNumber(index) => {
            format!(
                "Invalid number near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::BadKeyword(index) => {
            format!(
                "Invalid keyword name near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::BadIdentifier(index) => format!(
            "Invalid identifier name near {}",
            kdlite_error_near(*index, file_contents)
        ),
        kdlite::stream::Error::BadEscape(index) => format!(
            "Invalid escape sequence near {}",
            kdlite_error_near(*index, file_contents)
        ),
        kdlite::stream::Error::BadIndent(index) => {
            format!(
                "Invalid indentation near {}",
                kdlite_error_near(*index, file_contents)
            )
        }
        kdlite::stream::Error::MultipleChildren(index) => format!(
            "Multiple children for one KDL node near {}",
            kdlite_error_near(*index, file_contents)
        ),
        kdlite::stream::Error::UnexpectedEof => "Unexpected end of file".to_string(),
        kdlite::stream::Error::BannedChar(ch, index) => format!(
            "Invalid character `{}` near {}",
            ch.escape_default(),
            kdlite_error_near(*index, file_contents)
        ),
        _ => "Unknown error".to_string(),
    }
}

pub fn read_kdl_file(
    path: &Path,
) -> Result<kdlite::dom::Document<'static>, Box<dyn std::error::Error>> {
    let contents = std::fs::read_to_string(path)?;
    Ok(kdlite::dom::Document::parse(&contents)
        .map_err(|e| anyhow::anyhow!(display_kdlite_error(&e, &contents)))?
        .into_owned())
}
