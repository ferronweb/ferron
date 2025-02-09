use crate::project_karpacz_util::anti_xss::anti_xss;

pub fn generate_default_error_page(
  status_code: hyper::StatusCode,
  server_administrator_email: Option<&str>,
) -> String {
  let status_code_name = match status_code.canonical_reason() {
    Some(reason) => format!("{} {}", status_code.as_u16(), reason),
    None => format!("{}", status_code.as_u16()),
  };

  // Status code descriptions, many of which are directly taken from SVR.JS
  let error_500 = format!("The server had an unexpected error. You may need to contact the server administrator{} to resolve the error.", match server_administrator_email {
        Some(email_address) => format!(" at {}", email_address),
        None => String::from("")
    });
  let status_code_description = String::from(match status_code.as_u16() {
            200 => "The request succeeded! :)",
            201 => "A new resource has been created.",
            202 => "The request has been accepted for processing, but the processing has not been completed.",
            400 => "The request you made is invalid.",
            401 => "You need to authenticate yourself in order to access the requested file.",
            402 => "You need to pay in order to access the requested file.",
            403 => "You don't have access to the requested file.",
            404 => "The requested file doesn't exist. If you have typed the URL manually, then please check the spelling.",
            405 => "Method used to access the requested file isn't allowed.",
            406 => "The request is capable of generating only unacceptable content.",
            407 => "You need to authenticate yourself in order to use the proxy.",
            408 => "You have timed out.",
            409 => "The request you sent conflicts with the current state of the server.",
            410 => "The requested file is permanently deleted.",
            411 => "Content-Length property is required.",
            412 => "The server doesn't meet the preconditions you put in the request.",
            413 => "The request you sent is too large.",
            414 => "The URL you sent is too long.",
            415 => "The media type of request you sent isn't supported by the server.",
            416 => "The requested content range (Range header) you sent is unsatisfiable.",
            417 => "The expectation specified in the Expect property couldn't be satisfied.",
            418 => "The server (teapot) can't brew any coffee! ;)",
            421 => "The request you made isn't intended for this server.",
            422 => "The server couldn't process content sent by you.",
            423 => "The requested file is locked.",
            424 => "The request depends on another failed request.",
            425 => "The server is unwilling to risk processing a request that might be replayed.",
            426 => "You need to upgrade the protocols you use to request a file.",
            428 => "The request you sent needs to be conditional, but it isn't.",
            429 => "You sent too many requests to the server.",
            431 => "The request you sent contains headers that are too large.",
            451 => "The requested file isn't accessible for legal reasons.",
            497 => "You sent a non-TLS request to the HTTPS server.",
            500 => &error_500,
            501 => "The request requires the use of a function, which isn't currently implemented by the server.",
            502 => "The server had an error while it was acting as a gateway.",
            503 => "The service provided by the server is currently unavailable, possibly due to maintenance downtime or capacity problems. Please try again later.",
            504 => "The server couldn't get a response in time while it was acting as a gateway.",
            505 => "The server doesn't support the HTTP version used in the request.",
            506 => "The Variant header is configured to be engaged in content negotiation.",
            507 => "The server ran out of disk space necessary to complete the request.",
            508 => "The server detected an infinite loop while processing the request.",
            509 => "The server has its bandwidth limit exceeded.",
            510 => "The server requires an extended HTTP request. The request you made isn't an extended HTTP request.",
            511 => "You need to authenticate yourself in order to get network access.",
            598 => "The server couldn't get a response in time while it was acting as a proxy.",
            599 => "The server couldn't connect in time while it was acting as a proxy.",
        _ => "No description found for the status code."
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
