---
title: "Configuration: application backends"
description: "CGI, SCGI, and FastCGI directives for integrating dynamic application runtimes."
---

This page documents KDL directives for connecting Ferron to dynamic application runtimes over CGI, SCGI, and FastCGI.

## Directives

### CGI & application servers

- `cgi [enable_cgi: bool]` (_cgi_ module)
  - This directive specifies whether the CGI handler is enabled. Default: `cgi #false`
- `cgi_extension <cgi_extension: string|null>` (_cgi_ module)
  - This directive specifies CGI script extensions, which will be handled via the CGI handler outside the `cgi-bin` directory. This directive can be specified multiple times. Default: none
- `cgi_interpreter <cgi_extension: string> <cgi_interpreter: string|null> [<cgi_interpreter_argument: string> ...]` (_cgi_ module)
  - This directive specifies CGI script interpreters used by the CGI handler. If CGI interpreter is set to `#null`, the default interpreter settings will be disabled. This directive can be specified multiple times. Default: specified for `.pl`, `.py`, `.sh`, `.ksh`, `.csh`, `.rb` and `.php` extensions, and additionally `.exe`, `.bat` and `.vbs` extensions for Windows
- `cgi_environment <environment_variable_name: string> <environment_variable_value: string>` (_cgi_ module)
  - This directive specifies an environment variable passed into CGI applications. This directive can be specified multiple times. Default: none
- `scgi <scgi_to: string|null>` (_scgi_ module)
  - This directive specifies whether SCGI is enabled and the base URL to which the SCGI client will send requests. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `scgi #null`
- `scgi_environment <environment_variable_name: string> <environment_variable_value: string>` (_scgi_ module)
  - This directive specifies an environment variable passed into SCGI server. This directive can be specified multiple times. Default: none
- `fcgi <fcgi_to: string|null> [pass=<fcgi_pass: bool>]` (_fcgi_ module)
  - This directive specifies whether FastCGI is enabled and the base URL to which the FastCGI client will send requests. The `pass` prop specified whether to pass the all the requests to the FastCGI request handler. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `fcgi #null pass=#true`
- `fcgi_php <fcgi_php_to: string|null>` (_fcgi_ module)
  - This directive specifies whether PHP through FastCGI is enabled and the base URL to which the FastCGI client will send requests for ".php" files. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `fcgi_php #null`
- `fcgi_extension <fcgi_extension: string|null>` (_fcgi_ module)
  - This directive specifies file extensions, which will be handled via the FastCGI handle. This directive can be specified multiple times. Default: none
- `fcgi_environment <environment_variable_name: string> <environment_variable_value: string>` (_fcgi_ module)
  - This directive specifies an environment variable passed into FastCGI server. This directive can be specified multiple times. Default: none

## Configuration example

```kdl
cgi.example.com {
    // CGI configuration
    cgi
    cgi_extension ".cgi" ".pl" ".py"
    cgi_interpreter ".py" "/usr/bin/python3"
    cgi_interpreter ".pl" "/usr/bin/perl"
    cgi_environment "PATH" "/usr/bin:/bin"
    cgi_environment "SCRIPT_ROOT" "/var/www/cgi-bin"
}

scgi.example.com {
    // SCGI configuration
    scgi "tcp://localhost:4000/"
    scgi_environment "SCRIPT_NAME" "/app"
    scgi_environment "SERVER_NAME" "example.com"
}

fastcgi.example.com {
    // FastCGI configuration
    fcgi "tcp://localhost:9000/" pass=#true
    fcgi_php "tcp://localhost:9000/"
    fcgi_extension ".php" ".php5"
    fcgi_environment "DOCUMENT_ROOT" "/var/www/example.com"
}
```
