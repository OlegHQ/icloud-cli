pub mod form;
pub mod parse;
pub mod response_formatting;
/// curl - Transfer data from or to a server
pub mod types;

use self::form::generate_multipart_body;
use self::parse::parse_options;
use self::response_formatting::{apply_write_out, extract_filename, format_headers};
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::collections::HashMap;

pub struct CurlCommand;

/// Prepare request headers from options, including auth
fn prepare_headers(
    options: &types::CurlOptions,
    content_type: Option<&str>,
) -> HashMap<String, String> {
    let mut headers = HashMap::new();

    // Copy user-specified headers
    for (name, value) in &options.headers {
        headers.insert(name.clone(), value.clone());
    }

    // Add authentication header
    if let Some(ref user) = options.user {
        let encoded = STANDARD.encode(user.as_bytes());
        headers.insert("Authorization".to_string(), format!("Basic {}", encoded));
    }

    // Set content type if needed and not already set
    if let Some(ct) = content_type {
        if !headers.contains_key("Content-Type") {
            headers.insert("Content-Type".to_string(), ct.to_string());
        }
    }

    headers
}

/// Build output string from response
fn build_output(
    options: &types::CurlOptions,
    status: u16,
    resp_headers: &HashMap<String, String>,
    body: &str,
    request_url: &str,
    response_url: &str,
) -> String {
    let mut output = String::new();
    let status_text = default_status_text(status);

    // Verbose output
    if options.verbose {
        output.push_str(&format!("> {} {}\n", options.method, request_url));
        for (name, value) in &options.headers {
            output.push_str(&format!("> {}: {}\n", name, value));
        }
        output.push_str(">\n");
        output.push_str(&format!("< HTTP/1.1 {} {}\n", status, status_text));
        for (name, value) in resp_headers {
            output.push_str(&format!("< {}: {}\n", name, value));
        }
        output.push_str("<\n");
    }

    // Include headers with -i/--include
    if options.include_headers && !options.verbose {
        output.push_str(&format!("HTTP/1.1 {} {}\r\n", status, status_text));
        output.push_str(&format_headers(resp_headers));
        output.push_str("\r\n\r\n");
    }

    // Add body (unless head-only mode)
    if !options.head_only {
        output.push_str(body);
    } else if options.include_headers || options.verbose {
        // For HEAD, we already showed headers
    } else {
        // HEAD without -i shows headers
        output.push_str(&format!("HTTP/1.1 {} {}\r\n", status, status_text));
        output.push_str(&format_headers(resp_headers));
        output.push_str("\r\n");
    }

    // Write-out format
    if let Some(ref write_out) = options.write_out {
        output.push_str(&apply_write_out(
            write_out,
            status,
            resp_headers,
            response_url,
            body.len(),
        ));
    }

    output
}

fn default_status_text(status: u16) -> &'static str {
    http::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("Unknown")
}

#[async_trait]
impl Command for CurlCommand {
    fn name(&self) -> &'static str {
        "curl"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        // Parse options
        let options = match parse_options(&ctx.args) {
            Ok(opts) => opts,
            Err(e) => {
                return CommandResult::with_exit_code(String::new(), format!("{}\n", e), 2);
            }
        };

        // Check for URL
        let url_str = match options.url {
            Some(ref u) => u.clone(),
            None => {
                return CommandResult::with_exit_code(
                    String::new(),
                    "curl: no URL specified\n".to_string(),
                    2,
                );
            }
        };

        // Check for fetch_fn
        let fetch_fn = match ctx.fetch_fn {
            Some(ref f) => f.clone(),
            None => {
                return CommandResult::with_exit_code(
                    String::new(),
                    "curl: (6) Could not resolve host (network not available)\n".to_string(),
                    6,
                );
            }
        };

        // Normalize URL - add https:// if no protocol, then validate
        let url = if url::Url::parse(&url_str).is_err() {
            format!("https://{}", url_str)
        } else {
            url_str
        };

        // Prepare body
        let mut body: Option<String> = None;
        let mut content_type: Option<String> = None;

        // Handle -T/--upload-file
        if let Some(ref upload_file) = options.upload_file {
            let file_path = ctx.fs.resolve_path(&ctx.cwd, upload_file);
            match ctx.fs.read_file(&file_path).await {
                Ok(content) => body = Some(content),
                Err(_) => {
                    return CommandResult::with_exit_code(
                        String::new(),
                        format!("curl: (26) Failed to open/read file: {}\n", upload_file),
                        26,
                    );
                }
            }
        }

        // Handle -F/--form multipart data
        if !options.form_fields.is_empty() {
            let mut file_contents = HashMap::new();

            for field in &options.form_fields {
                if field.value.starts_with('@') || field.value.starts_with('<') {
                    let file_path = ctx.fs.resolve_path(&ctx.cwd, &field.value[1..]);
                    match ctx.fs.read_file(&file_path).await {
                        Ok(content) => {
                            file_contents.insert(field.value[1..].to_string(), content);
                        }
                        Err(_) => {
                            file_contents.insert(field.value[1..].to_string(), String::new());
                        }
                    }
                }
            }

            let (multipart_body, multipart_ct) =
                generate_multipart_body(&options.form_fields, &file_contents);
            body = Some(multipart_body);
            content_type = Some(multipart_ct);
        }

        // Handle -d/--data variants
        if body.is_none() {
            if let Some(ref data) = options.data {
                body = Some(data.clone());
            }
        }

        // Prepare headers
        let headers = prepare_headers(&options, content_type.as_deref());

        // Make the request
        match fetch_fn(url.clone(), options.method.clone(), headers, body).await {
            Ok(response) => {
                // Save cookies if requested
                if let Some(ref cookie_jar) = options.cookie_jar {
                    if let Some(set_cookie) = response.headers.get("set-cookie") {
                        let jar_path = ctx.fs.resolve_path(&ctx.cwd, cookie_jar);
                        let _ = ctx.fs.write_file(&jar_path, set_cookie.as_bytes()).await;
                    }
                }

                // Check for HTTP errors with -f/--fail
                if options.fail_silently && response.status >= 400 {
                    let stderr = if options.show_error || !options.silent {
                        format!(
                            "curl: (22) The requested URL returned error: {}\n",
                            response.status
                        )
                    } else {
                        String::new()
                    };
                    return CommandResult::with_exit_code(String::new(), stderr, 22);
                }

                let mut output = build_output(
                    &options,
                    response.status,
                    &response.headers,
                    &response.body,
                    &url,
                    &response.url,
                );

                // Write to file
                if options.output_file.is_some() || options.use_remote_name {
                    let filename = options
                        .output_file
                        .clone()
                        .unwrap_or_else(|| extract_filename(&url));
                    let file_path = ctx.fs.resolve_path(&ctx.cwd, &filename);
                    let file_body = if options.head_only {
                        ""
                    } else {
                        &response.body
                    };
                    let _ = ctx.fs.write_file(&file_path, file_body.as_bytes()).await;

                    // When writing to file, don't output body to stdout unless verbose
                    if !options.verbose {
                        output = String::new();
                    }

                    // Add write-out after file write
                    if let Some(ref write_out) = options.write_out {
                        output = apply_write_out(
                            write_out,
                            response.status,
                            &response.headers,
                            &response.url,
                            response.body.len(),
                        );
                    }
                }

                CommandResult::with_exit_code(output, String::new(), 0)
            }
            Err(message) => {
                let mut exit_code = 1;
                if message.contains("Network access denied") {
                    exit_code = 7;
                } else if message.contains("HTTP method") && message.contains("not allowed") {
                    exit_code = 3;
                } else if message.contains("Redirect target not in allow-list") {
                    exit_code = 47;
                } else if message.contains("Too many redirects") {
                    exit_code = 47;
                } else if message.contains("aborted") {
                    exit_code = 28;
                }

                let show_err = !options.silent || options.show_error;
                let stderr = if show_err {
                    format!("curl: ({}) {}\n", exit_code, message)
                } else {
                    String::new()
                };

                CommandResult::with_exit_code(String::new(), stderr, exit_code)
            }
        }
    }
}

#[cfg(test)]
mod tests;
