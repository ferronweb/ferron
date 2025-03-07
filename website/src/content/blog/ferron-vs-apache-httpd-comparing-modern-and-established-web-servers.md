---
title: "Ferron vs. Apache httpd: comparing modern and established web servers"
description: This blog post compares Ferron, a modern Rust-based web server, with the established Apache httpd. It covers various aspects, helping readers identify the strengths and ideal scenarios for each server to meet their needs.
date: 2025-03-07 15:20:00
cover: /img/covers/ferron-vs-apache-httpd-comparing-modern-and-established-web-servers.png
---
Ferron is a modern web server developed in Rust, that emphasizes performance, safety, and modularity. By using Rust's memory  safety features, Ferron aims to serve web content securely and efficiently.

Apache httpd on the other hand is a open-source web server software developed by the Apache Software Foundation. Since 1995, Apache httpd has played a very important role in the growth of World Wide Web, becoming one of the most widely used web servers in the world.

In this post, we will compare Ferron with Apache httpd, highlighting their design and architecture, performance, security, extensibility, configuration, use cases, community and support.

## Design and architecture

The design of a web server plays a very important role in determining its performance, security, and efficiency. Ferron, a modern web server developed in Rust, exemplifies a modular architecture that allows developers to extend its functionality through various server modules. Built using the Tokio event-driven runtime, Ferron's architecture enables it to handle asynchronous operations efficiently. This modularity enables Ferron to be highly customizable and adaptable to different use cases. Similarly, the Apache httpd, a widely-used open-source web server, boasts a modular architecture that allows administrators to customize the server by enabling or disabling modules to suit their specific needs. Both Ferron and Apache httpd demonstrate the importance of modularity in web server design, enabling them to be flexible, reliable, and robust.

The architecture of web servers like Ferron and Apache httpd also highlights the significance of performance-enhancing features such as asynchronous programming, load balancing, and caching mechanisms. Ferron's use of Rust's asynchronous programming capabilities enables it to handle requests efficiently and quickly, while Apache httpd's Multi-Processing Modules (MPMs) allow the server to operate in various modes, including process-based, threaded, or a hybrid model. Additionally, both servers support features like reverse and forward proxy configurations, load balancing, and SSL/TLS support, making them versatile and powerful tools for managing web traffic and ensuring high availability and performance. By understanding the design and architecture of web servers like Ferron and Apache, developers can create more efficient, secure, and scalable web serving solutions.

## Performance

![A chart comparing the performance of various web servers, including Ferron and Apache httpd](/img/performance-comparison-chart.png)

The performance of static web servers, as illustrated above, was evaluated using the command `wrk -c 100 -d 60 -t 6 https://localhost/index.nginx-debian.html` on a server equipped with an AMD Ryzen 5 8600G CPU. The benchmarked web servers served the default NGINX web server page, and all configurations included request logging to a log file.

The results of the benchmarks, presented above, indicate that Ferron achieved around 143,000 requests per second, while Apache httpd with the pre-fork MPM handled around 120,000 requests per second. In contrast, Apache httpd with the event MPM managed around 92,000 requests per second.

These results highlight the performance differences between web servers and their configurations, emphasizing the efficiency of Ferron in handling high request loads. The significant gap between Apache httpd’s pre-fork and event MPM suggests that the latter’s asynchronous handling may introduce overhead in this specific test environment. Additionally, factors such as process management, connection handling, and request logging could contribute to these variations. Further testing with different workloads and concurrency levels would provide deeper insights into the scalability and efficiency of each server.

## Security

When evaluating the security aspects of web servers, it's essential to consider their features, extensibility, and the programming languages in which they are implemented. Below is an overview of the security features of both Ferron and Apache HTTP Server (httpd).

Ferron is a modern web server developed in Rust, a programming language known for its emphasis on memory safety and concurrency. Rust's design inherently prevents common vulnerabilities such as buffer overflows and null pointer dereferencing, contributing to a more secure foundation for Ferron. Specific security features of Ferron include:

- **Memory safety** - Rust's ownership model ensures that memory management errors, which can lead to security vulnerabilities, are minimized.
- **Concurrency** - Rust's concurrency model allows Ferron to handle multiple requests efficiently without compromising safety, reducing the risk of race conditions.
- **Modular architecture** - Ferron's design allows for the integration of security-focused modules, enabling administrators to tailor security measures to their specific needs.
- **Secure defaults** - emphasizing secure default configurations, Ferron aims to minimize potential attack surfaces out of the box.

Apache httpd on the other hand is a widely used web server with a long-standing reputation for flexibility and extensive feature support. Its security features are comprehensive and include:

- **Authentication and authorization modules** - Apache httpd provides various modules for authentication (e.g., `mod_auth_basic`, `mod_auth_digest`) and authorization, allowing fine-grained access control based on user credentials and group memberships.
- **Access control** - administrators can restrict access to specific directories or files based on IP addresses or other criteria, enhancing security by limiting exposure.
- **SSL/TLS support** - through modules like `mod_ssl`, Apache httpd supports SSL/TLS protocols, enabling encrypted communications between clients and the server.
- **ModSecurity integration** - Apache httpd can be extended with ModSecurity, an open-source web application firewall, providing real-time monitoring, logging, and access control to mitigate various attacks.
- **Server Side Includes (SSI) control** - the `mod_include` module manages SSI, allowing dynamic content generation while providing mechanisms to control and secure its usage.
- **URL rewriting and filtering** - modules like `mod_rewrite` and `mod_filter` enable administrators to manipulate URLs and content, facilitating security measures such as input validation and request filtering.
- **Comprehensive logging** - Apache httpd's logging capabilities, managed by modules like `mod_log_config`, allow detailed monitoring of server activity, aiding in the detection and analysis of potential security incidents.

Both Ferron and Apache httpd prioritize security but approach it differently based on their design philosophies and architectures. Ferron's implementation in Rust provides inherent memory safety and modern concurrency features, reducing certain classes of vulnerabilities. Apache's extensive module ecosystem offers a wide array of configurable security features, allowing administrators to implement tailored security measures. The choice between the two may depend on specific security requirements, performance considerations, and familiarity with the respective ecosystems. 

## Modules and extensibility

When comparing the extensibility and module ecosystems of Ferron and Apache httpd, it's important to consider their architectures and the range of available modules.

Ferron is a modern web server developed in Rust, emphasizing performance, safety, and modularity. Its modular architecture allows for the integration of various functionalities through server modules, enabling customization and extension of its capabilities. However, as a relatively new entrant in the web server landscape, Ferron's module ecosystem is still growing, and the range of available modules may be limited compared to more established servers.

Apache httpd on the other hand boasts a long-standing reputation for flexibility and extensibility, largely due to its modular design. It supports a vast array of modules, both official and third-party, that enhance its functionality. Some notable modules include:

- **mod_ssl** - provides support for SSL and TLS protocols, enabling secure communications.
- **mod_proxy** - offers proxy capabilities, allowing Apache to function as a forward or reverse proxy server.
- **mod_rewrite** - enables flexible URL rewriting and manipulation, useful for creating user-friendly URLs and implementing redirects.
- **mod_auth_basic** and **mod_auth_digest** - facilitate basic and digest authentication mechanisms, respectively, for controlling access to resources.
- **mod_perl** - embeds a Perl interpreter within the server, allowing for dynamic content generation and advanced server-side scripting.
- **mod_lua** - integrates the Lua scripting language, enabling lightweight and efficient scripting capabilities.
- **mod_wsgi** - allows for hosting Python-based web applications by implementing the Web Server Gateway Interface (WSGI).

This extensive module ecosystem allows Apache httpd to be customized to a wide range of use cases, from serving static content to acting as a reverse proxy or load balancer. The availability of numerous third-party modules further enhances its adaptability, allowing integration with various technologies and platforms.

## Configuration and ease of use

When comparing the configuration and ease of use of Ferron and Apache HTTP Server (httpd), it's essential to consider their configuration formats, flexibility, and user-friendliness.

Ferron utilizes YAML for its configuration files. YAML is known for its readability and simplicity, using indentation to represent hierarchical relationships, which can make configurations more intuitive and easier to manage. This approach allows administrators to quickly grasp and modify settings without dealing with complex syntax.

Apache httpd on the other hand uses a text-based configuration system with a hierarchical structure. The primary configuration file is `httpd.conf`, which contains directives governing the server's global environment, default settings for the main server, and configurations for virtual hosts. Additionally, per-directory settings can be managed using `.htaccess` files, allowing for localized configuration adjustments.

Apache httpd's configuration language is based on blocks of directives, offering granular control over various server aspects. While this provides flexibility, the syntax can be intricate, and the extensive range of available directives may present a learning curve for new users. However, Apache httpd's long-standing presence means there is abundant documentation and community support to assist administrators in configuring and optimizing the server.

Ferron's use of YAML for configuration emphasizes simplicity and readability, potentially reducing the time required to set up and maintain the server. This can be particularly beneficial for users who prefer straightforward configurations or are new to web server administration.

In contrast, Apache httpd's configuration system offers extensive flexibility and control, accommodating a wide range of use cases through its modular design. While this complexity allows for fine-tuned configurations, it may require a more in-depth understanding of the server's directives and modules. However, the vast amount of documentation and community resources available can mitigate this challenge, providing guidance and best practices for various scenarios.

## Use cases and deployment scenarios

When considering the use cases and deployment scenarios for web servers, it's important to assess their features, performance characteristics, and suitability for various environments.

Ferron is a modern web server written in Rust, emphasizing speed, memory safety, and efficiency. Its design makes it particularly suitable for scenarios where performance and security are paramount. The deployment scenarios for Ferron include:

- **High-performance applications** - Ferron's architecture is optimized for handling numerous simultaneous connections with minimal latency, making it ideal for applications requiring rapid response times.
- **Security-focused environments** - leveraging Rust's memory safety features, Ferron reduces common vulnerabilities such as buffer overflows, aligning with the needs of security-sensitive deployments.
- **Modern web services** - for projects seeking to utilize contemporary technologies and benefit from a modular, extensible server design, Ferron offers a forward-looking platform.

As Ferron is relatively new, its ecosystem is still developing. Organizations adopting Ferron may need to invest in building custom modules or tools to fully leverage its capabilities.

Apache httpd on the other hand is a longstanding, versatile web server known for its robustness and extensive feature set. Its flexibility allows it to be deployed in a wide array of scenarios, including:

- **Traditional web hosting** - Apache httpd's support for various content types and authentication mechanisms makes it a staple in conventional web hosting environments.
- **Dynamic content delivery** - through modules like `mod_php` and `mod_perl`, Apache efficiently serves dynamic content generated by server-side scripting languages.
- **Reverse proxy and load balancing** - with modules such as `mod_proxy`, Apache can distribute incoming traffic across multiple backend servers, enhancing scalability and reliability.
- **Cross-platform deployments** - Apache httpd's compatibility with various operating systems, including Windows and Unix-like systems, allows for flexible deployment across diverse infrastructures.
- **Embedded systems** - due to its configurable footprint, Apache httpd can be tailored for use in embedded systems where resources are constrained.

Apache httpd's extensive module ecosystem and community support make it adaptable to numerous deployment scenarios. However, its performance may not match that of more specialized servers in high-concurrency situations without careful tuning.

## Community and support

When evaluating the community support and resources available for Ferron and Apache httpd, it's important to consider the maturity and engagement of their respective communities.

Ferron is a relatively new web server project, introduced as a fast, memory-safe server written in Rust. Its development is ongoing, with a focus on refining performance and expanding use cases. As Ferron approaches its beta release, the development team emphasizes enhancing documentation and sharing updates to assist users in adopting the server. Given its recent emergence, Ferron's community is still growing, and users may find limited resources and community-driven support channels at this stage.

Apache httpd on the other hand has a long-standing and active community. Launched in 1995, Apache has become one of the most widely used web servers globally. The Apache Software Foundation oversees its development, ensuring a structured and collaborative environment. Users can access a wealth of resources, including comprehensive documentation, mailing lists, and forums, to seek assistance and engage with other users. The project's maturity and widespread adoption have fostered a robust community that contributes to continuous improvement and support.

## Conclusion

In conclusion, both Ferron and Apache httpd offer robust solutions for web serving, each with its unique strengths and ideal use cases. Ferron, as a modern web server developed in Rust, excels in performance and security, leveraging Rust's memory safety features and asynchronous programming capabilities. Its modular architecture and emphasis on secure defaults make it a compelling choice for high-performance and security-focused environments. However, as a newer entrant, Ferron's ecosystem and community support are still developing, which may present challenges for some web server administrators.

Apache httpd, on the other hand, stands as a pillar of web server technology, boasting a long history of reliability and an extensive feature set. Its modular design, comprehensive security features, and vast array of available modules make it highly adaptable to various use cases, from traditional web hosting to complex dynamic content delivery. Apache's mature community and abundant resources provide a strong support system for users, ensuring continuous improvement and widespread adoption.

The choice between Ferron and Apache httpd ultimately depends on specific needs and priorities. Organizations seeking cutting-edge performance and security may find Ferron's modern approach appealing, while those requiring extensive flexibility and established community support may opt for Apache httpd. Both servers demonstrate the importance of modularity, performance, and security in web server design, offering powerful tools for managing web traffic efficiently and securely.