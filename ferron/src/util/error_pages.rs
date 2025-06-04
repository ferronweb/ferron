use super::anti_xss;

/// Generates a default error page
pub fn generate_default_error_page(
  status_code: hyper::StatusCode,
  server_administrator_email: Option<String>,
) -> String {
  let status_code_name = match status_code.canonical_reason() {
    Some(reason) => format!("{} {}", status_code.as_u16(), reason),
    None => format!("{}", status_code.as_u16()),
  };

  let error_500 = format!("The server encountered an unexpected error. You may need to contact the server administrator{} to resolve the error.", match server_administrator_email {
        Some(email_address) => format!(" at {}", email_address),
        None => String::from("")
    });
  let status_code_description = String::from(match status_code.as_u16() {
    200 => "The request was successful!",
    201 => "A new resource was successfully created.",
    202 => "The request was accepted but hasn't been fully processed yet.",
    400 => "The request was invalid.",
    401 => "Authentication is required to access the resource.",
    402 => "Payment is required to access the resource.",
    403 => "You're not authorized to access this resource.",
    404 => "The requested resource wasn't found. Double-check the URL if entered manually.",
    405 => "The request method is not allowed for this resource.",
    406 => "The server cannot provide a response in an acceptable format.",
    407 => "Proxy authentication is required.",
    408 => "The request took too long and timed out.",
    409 => "There's a conflict with the current state of the server.",
    410 => "The requested resource has been permanently removed.",
    411 => "The request must include a Content-Length header.",
    412 => "The request doesn't meet the server's preconditions.",
    413 => "The request is too large for the server to process.",
    414 => "The requested URL is too long.",
    415 => "The server doesn't support the request's media type.",
    416 => "The requested content range is invalid or unavailable.",
    417 => "The expectation in the Expect header couldn't be met.",
    418 => "This server (a teapot) refuses to make coffee! ☕",
    421 => "The request was directed to the wrong server.",
    422 => "The server couldn't process the provided content.",
    423 => "The requested resource is locked.",
    424 => "The request failed due to a dependency on another failed request.",
    425 => "The server refuses to process a request that might be replayed.",
    426 => "The client must upgrade its protocol to proceed.",
    428 => "A precondition is required for this request, but it wasn't included.",
    429 => "Too many requests were sent in a short period.",
    431 => "The request headers are too large.",
    451 => "Access to this resource is restricted due to legal reasons.",
    497 => "A non-TLS request was sent to an HTTPS server.",
    500 => &error_500,
    501 => "The server doesn't support the requested functionality.",
    502 => "The server, acting as a gateway, received an invalid response.",
    503 => {
      "The server is temporarily unavailable (e.g., maintenance or overload). Try again later."
    }
    504 => "The server, acting as a gateway, timed out waiting for a response.",
    505 => "The HTTP version used in the request isn't supported.",
    506 => "The Variant header caused a content negotiation loop.",
    507 => "The server lacks sufficient storage to complete the request.",
    508 => "The server detected an infinite loop while processing the request.",
    509 => "Bandwidth limit exceeded on the server.",
    510 => "The server requires an extended HTTP request, but the client didn't send one.",
    511 => "Authentication is required to access the network.",
    598 => "The proxy server didn't receive a response in time.",
    599 => "The proxy server couldn't establish a connection in time.",
    _ => "No description found for the status code.",
  });

  format!(
    "<!DOCTYPE html>
<html lang=\"en\">
<head>
    <meta charset=\"UTF-8\">
    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">
    <title>{}</title>
</head>
<body>
    <h1>{}</h1>
    <p>{}</p>
</body>
</html>",
    anti_xss(&status_code_name),
    anti_xss(&status_code_name),
    anti_xss(&status_code_description)
  )
}
