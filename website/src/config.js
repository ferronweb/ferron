export const headerLinks = {
  nav: [
    {
      href: "/",
      target: "_self",
      label: "Home"
    },
    {
      href: "/docs",
      target: "_self",
      label: "Docs"
    },
    {
      href: "/blog",
      target: "_self",
      label: "Blog"
    }
  ],
  github: {
    href: "https://github.com/ferronweb/ferron",
    target: "_blank"
  }
};

export const questions = [
  {
    key: "item-1",
    question: "What is a web server?",
    answer:
      "A web server is hardware device or software application that delivers web pages to clients over the internet."
  },
  {
    key: "item-2",
    question: "What is Ferron?",
    answer:
      "Ferron is a fast, memory-safe web server written in Rust. It supports both static files, and dynamic content (including PHP). Ferron is licensed under a permissive MIT license."
  },
  {
    key: "item-3",
    question: "How was Ferron created?",
    answer:
      'The idea of developing a web server in a systems programming language arose when Dorian Niemiec, the creator of SVR.JS, found its Node.js implementation too slow based on performance benchmarks. He first experimented with various programming languages for web servers, and found Rust to be efficient enough. Then he experimented with building a custom low-level HTTP handler, but encountered conflicts with Rust\'s borrow checker, leading to lifetime issues, so he used Hyper instead. The end result became "Project Karpacz" (named after a city in Poland near the mountains), and later "Ferron".'
  },
  {
    key: "item-4",
    question: "How did Ferron get its name?",
    answer:
      'The name "Ferron" is derived from "ferrous," referencing Rust\'s iron theme. Rust programming language is actually named after the rust fungus.'
  },
  {
    key: "item-5",
    question: "How can I use Ferron?",
    answer: "You can read the documentation to learn how to use Ferron."
  }
];

export const footerLinks = {
  otherPages: [
    { href: "/", label: "Home" },
    { href: "/download", label: "Download" },
    { href: "/contact", label: "Contact" },
    { href: "/blog", label: "Blog" }
  ],
  resources: [
    { href: "/docs", label: "Docs" },
    { href: "/changelog", label: "Change log" },
    {
      href: "https://github.com/ferronweb/ferron",
      label: "GitHub repository"
    },
    { href: "/contribute", label: "Contribute" },
    { href: "/vulnerabilities", label: "Vulnerabilities" }
  ],
  footerBottom: {
    termsofService: {
      href: "/tos",
      label: "Terms of Service"
    },
    privacyPolicy: {
      href: "/privacy",
      label: "Privacy Policy"
    },
    rightsReserved: {
      href: "https://www.ferronweb.org/",
      label: "Ferron"
    }
  }
};
