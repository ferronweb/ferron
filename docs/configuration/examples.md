---
title: "Configuration: configuration examples"
description: "Complete KDL configuration examples for language negotiation, locations, and multi-section real-world setups."
---

This page collects practical KDL configuration examples you can adapt for common Ferron deployment patterns.

## Language-specific content configuration example

Below is an example of Ferron configuration involving conditionals to serve language-specific content:

```kdl
// Snippet for common language settings
snippet "LANG_COMMON" {
    set_constant "LANGUAGES" "en,de,pl"
}

example.com {
    // Generic response
    status 200 body="lang: Unknown"

    condition "LANG_PL" {
        use "LANG_COMMON"
        is_language "pl"
    }

    condition "LANG_DE" {
        use "LANG_COMMON"
        is_language "de"
    }

    condition "LANG_EN" {
        use "LANG_COMMON"
        is_language "en"
    }

    if "LANG_PL" {
        // Polish language
        status 200 body="lang: Polski"
    }

    if "LANG_DE" {
        // German language
        status 200 body="lang: Deutsch"
    }

    if "LANG_EN" {
        // English language
        status 200 body="lang: English"
    }
}
```

## Location block example

Below is an example of Ferron configuration involving location blocks:

```kdl
example.com {
    root "/var/www/example.com"

    // Static assets with different settings
    location "/static" remove_base=#true {
        root "/var/www/static"
        compressed
        file_cache_control "public, max-age=31536000"
    }

    // API endpoints with proxy
    location "/api" {
        proxy "http://backend:8080"
        proxy_request_header "X-Forwarded-For" "{client_ip}"
        limit rate=50 burst=100
    }

    // Admin area with authentication
    location "/admin" {
        status 401 realm="Admin Access" users="admin"
        root "/var/www/admin"
    }

    // PHP files
    fcgi_php "tcp://localhost:9000/"
}
```

## Complete example combining multiple sections

Below is a complete example of Ferron configuration, combining multiple sections:

```kdl
// Global configuration
* {
    // Protocol and performance settings
    protocols "h1" "h2"
    h2_initial_window_size 65536
    h2_max_concurrent_streams 100
    timeout 300000

    // Network settings
    listen_ip "0.0.0.0"
    default_http_port 80
    default_https_port 443

    // Security defaults
    tls_cipher_suite "TLS_AES_256_GCM_SHA384" "TLS_AES_128_GCM_SHA256"
    ocsp_stapling
    block "192.168.1.100"

    // Global caching
    cache_max_entries 1024

    // Load balancing settings
    lb_health_check_window 5000

    // Logging
    log "/var/log/ferron/access.log"
    error_log "/var/log/ferron/error.log"

    // Static file defaults
    compressed
    etag
}

// Define reusable snippets
snippet "security_headers" {
    // Common security headers
    header "X-Frame-Options" "DENY"
    header "X-Content-Type-Options" "nosniff"
    header "X-XSS-Protection" "1; mode=block"
    header "Referrer-Policy" "strict-origin-when-cross-origin"
}

snippet "cors_headers" {
    // CORS headers for API endpoints
    header "Access-Control-Allow-Origin" "*"
    header "Access-Control-Allow-Methods" "GET, POST, PUT, DELETE, OPTIONS"
    header "Access-Control-Allow-Headers" "Authorization, Content-Type, X-Requested-With"
    header "Access-Control-Max-Age" "86400"
}

snippet "static_caching" {
    // Aggressive caching for static assets
    file_cache_control "public, max-age=31536000, immutable"
    compressed
    etag
}

snippet "admin_protection" {
    // Admin area protection
    status 401 realm="Admin Area" users="admin,superuser"
    user "admin" "$2b$10$hashedpassword12345"
    user "superuser" "$2b$10$anotherhashpassword67890"
    limit rate=10 burst=20
}

snippet "mobile_condition" {
    condition "is_mobile" {
        is_regex "{header:User-Agent}" "(Mobile|Android|iPhone|iPad)" case_insensitive=#true
    }
}

snippet "admin_ip_condition" {
    condition "is_admin_ip" {
        is_remote_ip "192.168.1.10" "10.0.0.5"
    }
}

snippet "api_request_condition" {
    condition "is_api_request" {
        is_regex "{path}" "^/api/"
    }
}

snippet "static_asset_condition" {
    condition "is_static_asset" {
        is_regex "{path}" "\\.(css|js|png|jpg|jpeg|gif|svg|woff|woff2|ttf|eot|ico)(?:$|[?#])" case_insensitive=#true
    }
}

snippet "development_condition" {
    condition "is_development" {
        is_equal "{header:X-Environment}" "development"
    }
}

// Main website with conditional configuration
example.com {
    // TLS configuration
    tls "/etc/ssl/certs/example.com.crt" "/etc/ssl/private/example.com.key"
    auto_tls_contact "admin@example.com"

    // Basic settings
    root "/var/www/example.com"
    server_administrator_email "admin@example.com"

    // Use security headers snippet
    use "security_headers"

    // Import condition snippets
    use "mobile_condition"
    use "admin_ip_condition"
    use "api_request_condition"
    use "static_asset_condition"
    use "development_condition"

    // Additional security header
    header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

    // Conditional configuration based on mobile detection
    if "is_mobile" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Mobile-specific settings
        header "X-Mobile-Detected" "true"
        root "/var/www/example.com/mobile"

        // Lighter rate limiting for mobile
        limit rate=50 burst=100
    }

    if_not "is_mobile" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Desktop settings
        header "X-Mobile-Detected" "false"
        limit rate=100 burst=200
    }

    // Admin IP gets special treatment
    if "is_admin_ip" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // No rate limiting for admin IPs
        limit #false

        // Additional debug headers
        header "X-Admin-Access" "true"
        header "X-Client-IP" "{client_ip}"

        // Enhanced logging for admin access
        log "/var/log/ferron/admin-access.log"
    }

    // Development environment conditional settings
    if "is_development" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Development-specific headers
        header "X-Environment" "development"
        header "X-Debug-Mode" "enabled"

        // Disable caching in development
        header "Cache-Control" "no-cache, no-store, must-revalidate"
        header "Pragma" "no-cache"
        header "Expires" "0"
    }

    if_not "is_development" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Production caching
        cache
        cache_vary "Accept-Encoding" "Accept-Language"
        file_cache_control "public, max-age=3600"
    }

    // URL rewriting
    rewrite "^/old-section/(.*)" "/new-section/$1" last=#true

    // Error pages
    error_page 404 "/var/www/errors/404.html"
    error_page 500 "/var/www/errors/500.html"

    // Static assets location with conditional caching
    location "/assets" remove_base=#true {
        root "/var/www/assets"

        if "is_static_asset" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            use "static_caching"
        }

        // CORS for web fonts
        if_not "is_static_asset" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            header "Access-Control-Allow-Origin" "*"
        }
    }

    // API endpoints
    location "/api" {
        if "is_api_request" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            use "cors_headers"

            // API-specific rate limiting
            limit rate=1000 burst=2000

            // Proxy to API backend
            proxy "http://api-backend:8080"
            proxy_request_header_replace "X-Real-IP" "{client_ip}"
            proxy_request_header "X-Forwarded-Proto" "{scheme}"
        }
    }

    // Admin area with conditional access
    location "/admin" {
        if "is_admin_ip" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            // Admin IPs get direct access
            root "/var/www/admin"

            // Special admin headers
            header "X-Admin-Direct-Access" "true"
        }

        if_not "is_admin_ip" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            // Non-admin IPs require authentication
            use "admin_protection"
        }
    }

    // PHP application
    fcgi_php "tcp://localhost:9000/"
}

// API subdomain with extensive conditional logic
api.example.com {
    // TLS configuration
    tls "/etc/ssl/certs/api.example.com.crt" "/etc/ssl/private/api.example.com.key"

    // Use CORS headers snippet
    use "cors_headers"

    // Import reusable condition
    use "admin_ip_condition"

    // Conditional backend selection based on request path
    condition "is_v1_api" {
        is_regex "{path}" "^/v1/"
    }

    condition "is_v2_api" {
        is_regex "{path}" "^/v2/"
    }

    condition "is_auth_request" {
        is_regex "{path}" "^/(login|register|refresh|logout)"
    }

    // Version-specific backend routing
    if "is_v1_api" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        proxy "http://api-v1-backend1:8080"
        proxy "http://api-v1-backend2:8080"

        // V1 specific headers
        header "X-API-Version" "1.0"

        // More restrictive rate limiting for legacy API
        limit rate=500 burst=1000
    }

    if "is_v2_api" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        proxy "http://api-v2-backend1:8080"
        proxy "http://api-v2-backend2:8080"
        proxy "http://api-v2-backend3:8080"

        // V2 specific headers
        header "X-API-Version" "2.0"

        // Higher rate limits for new API
        limit rate=2000 burst=4000
    }

    // Authentication endpoints get special treatment
    if "is_auth_request" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        // Route to dedicated auth service
        proxy "http://auth-service:9090"

        // Stricter rate limiting for auth endpoints
        limit rate=100 burst=200

        // Additional security headers
        header "X-Auth-Endpoint" "true"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains; preload"
    }

    // Admin IP gets enhanced access
    if "is_admin_ip" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        // No rate limiting for admin IPs
        limit #false

        // Admin-specific headers
        header "X-Admin-API-Access" "true"

        // Enhanced logging
        log "/var/log/ferron/api-admin.log"
    }

    // Health checking
    lb_health_check
    lb_health_check_max_fails 3

    // Proxy settings
    proxy_keepalive
    proxy_request_header_replace "X-Real-IP" "{client_ip}"
    proxy_request_header "X-Forwarded-Proto" "{scheme}"

    // Health check endpoint
    status 200 url="/health" body="OK"

    // Version info endpoint
    status 200 url="/version" body="{\"api_versions\":[\"v1\",\"v2\"],\"server\":\"Ferron\"}"
}

// Development subdomain with environment-based configuration
dev.example.com {
    // Basic TLS
    auto_tls
    auto_tls_contact "dev@example.com"

    // Use security headers but with relaxed settings
    use "security_headers"

    // Import reusable condition
    use "admin_ip_condition"

    // Override some security headers for development
    header "X-Frame-Options" "SAMEORIGIN"

    // Enhanced logging
    log "/var/log/ferron/dev.access.log"
    error_log "/var/log/ferron/dev.error.log"

    // Development-specific conditions
    condition "is_hot_reload" {
        is_regex "{path}" "^/(sockjs-node|__webpack_hmr)"
    }

    condition "is_source_map" {
        is_regex "{path}" "\\.map(?:$|[?#])"
    }

    // Hot reload support
    if "is_hot_reload" {
        // Non-inherited headers, again
        use "security_headers"
        header "X-Frame-Options" "SAMEORIGIN"

        proxy "http://dev-hmr:3001"

        // WebSocket support
        proxy_request_header "Connection" "Upgrade"
        proxy_request_header "Upgrade" "websocket"

        // No caching for hot reload
        header "Cache-Control" "no-cache"
    }

    // Source maps handling
    if "is_source_map" {
        // Only allow source maps for admin IPs
        if "is_admin_ip" {
            root "/var/www/dev/sourcemaps"
        }

        if_not "is_admin_ip" {
            status 404 body="Not found"
        }
    }

    // Test endpoints with conditional responses
    if "is_admin_ip" {
        status 200 url="/test" body="Development server is working (Admin Access)"
        status 200 url="/debug" body="{\"client_ip\":\"{client_ip}\",\"method\":\"{method}\",\"path\":\"{path}\"}"
    }

    if_not "is_admin_ip" {
        status 200 url="/test" body="Development server is working"
    }

    // Default proxy to development backend
    proxy "http://dev-backend:3000"
    proxy_request_header "X-Dev-Mode" "true"
    proxy_request_header "X-Environment" "development"

    // Relaxed rate limiting
    limit rate=1000 burst=5000
}

// Static content CDN with intelligent caching
cdn.example.com {
    // TLS configuration
    tls "/etc/ssl/certs/cdn.example.com.crt" "/etc/ssl/private/cdn.example.com.key"

    // Static file serving
    root "/var/www/cdn"
    directory_listing #false

    // Use static caching snippet
    use "static_caching"

    // Import reusable condition
    use "admin_ip_condition"

    // Conditional caching based on file type
    condition "is_image" {
        is_regex "{path}" "\\.(png|jpg|jpeg|gif|webp|svg)(?:$|[?#])" case_insensitive=#true
    }

    condition "is_font" {
        is_regex "{path}" "\\.(woff|woff2|ttf|eot|otf)(?:$|[?#])" case_insensitive=#true
    }

    condition "is_media" {
        is_regex "{path}" "\\.(mp4|webm|ogg|mp3|wav)(?:$|[?#])" case_insensitive=#true
    }

    // Image-specific settings
    if "is_image" {
        // Extra long caching for images
        file_cache_control "public, max-age=2592000, immutable"

        // Image-specific headers
        header "X-Content-Type" "image"
    }

    // Font-specific settings
    if "is_font" {
        // CORS for web fonts
        header "Access-Control-Allow-Origin" "*"
        header "Access-Control-Allow-Methods" "GET, HEAD, OPTIONS"

        // Font-specific caching
        file_cache_control "public, max-age=31536000, immutable"
    }

    // Media-specific settings
    if "is_media" {
        // Partial content support for media
        header "Accept-Ranges" "bytes"

        // Media-specific caching
        file_cache_control "public, max-age=604800"
    }

    // Admin IP gets special access
    if "is_admin_ip" {
        // Allow directory listing for admin IPs
        directory_listing #true

        // Admin headers
        header "X-Admin-CDN-Access" "true"
    }

    // No rate limiting for CDN
    limit #false

    // Geographic optimization (example condition)
    condition "is_european_request" {
        is_regex "{header:CF-IPCountry}" "^(DE|FR|GB|IT|ES|NL|BE|CH|AT|SE|NO|DK|FI|PL)$"
    }

    if "is_european_request" {
        header "X-CDN-Region" "Europe"
        // Could proxy to European CDN nodes here
    }

    if_not "is_european_request" {
        header "X-CDN-Region" "Global"
    }
}
```
