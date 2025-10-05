extern crate cgi;

cgi::cgi_main! { |request: cgi::Request| -> cgi::Response {
     cgi::text_response(200, "Hello, World!")
} }
