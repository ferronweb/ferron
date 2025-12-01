# Module development notes

Ferron can be extended with modules, which handle HTTP requests, DNS providers for DNS-01 ACME challenges, and observability backends. All of them are essentially Rust crates.

You can find both the [example Ferron module](https://github.com/ferronweb/ferron-module-example), the [example Ferron DNS provider](https://github.com/ferronweb/ferron-dns-mock), and the [example Ferron observability backend support](https://github.com/ferronweb/ferron-observability-example).
