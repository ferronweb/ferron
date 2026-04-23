use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- determine Markdown files in docs directory ---
    let docs_files = glob::glob("./docs/**/*.md")?;

    for file in docs_files {
        // --- read and parse each Markdown file ---
        let file = file?;
        let contents = std::fs::read_to_string(&file)?;
        let parsed = markdown::to_mdast(&contents, &markdown::ParseOptions::gfm())
            .map_err(|e| anyhow::anyhow!(e))?;

        // --- find "ferron" code blocks in the parsed Markdown ---
        let ferron_code_blocks = find_ferron_code_blocks(&parsed);
        for (block, pos) in ferron_code_blocks {
            // --- write temporary config file with the code block ---
            let temp_file = tempfile::NamedTempFile::new()?;
            std::fs::write(&temp_file, block)?;

            // --- run ferron with the temporary config file ---
            let output = std::process::Command::new(get_ferron_bin_path())
                .arg("validate")
                .arg("-c")
                .arg(temp_file.path())
                .arg("--config-adapter")
                .arg("ferronconf")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null())
                .spawn()?
                .wait_with_output()?;
            if !output.status.success() || !output.stderr.is_empty() {
                Err(anyhow::anyhow!(
                    "Configuration validation (in {}{}) failed with status: {}\n{}",
                    file.display(),
                    pos.map_or(String::new(), |p| format!(
                        " beginning at line {}, column {}",
                        p.start.line, p.start.column
                    )),
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                ))?;
            }
        }
    }

    println!("All documentation tests passed.");

    Ok(())
}

/// Finds "ferron" code blocks in the parsed Markdown.
/// Returns a vector of the code block values.
fn find_ferron_code_blocks(
    parsed: &markdown::mdast::Node,
) -> Vec<(String, Option<markdown::unist::Position>)> {
    let mut result = Vec::new();

    if let markdown::mdast::Node::Code(code_block) = &parsed {
        if code_block.lang.as_deref() == Some("ferron") {
            result.push((code_block.value.clone(), code_block.position.clone()));
        }
    }

    if let markdown::mdast::Node::Root(root) = parsed {
        for child in &root.children {
            result.extend(find_ferron_code_blocks(child));
        }
    }

    result
}

fn get_ferron_bin_path() -> &'static str {
    if fs::exists("target/debug/ferron").unwrap_or(false) {
        "target/debug/ferron"
    } else {
        "target/release/ferron"
    }
}
