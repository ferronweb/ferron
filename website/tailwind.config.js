/** @type {import('tailwindcss').Config} */
export default {
  darkMode: ["class"],
  content: ["./src/**/*"],
  theme: {
    extend: {
      colors: {
        border: "hsla(var(--border), 1)",
        input: "hsla(var(--input), 1)",
        ring: "hsla(var(--ring), 1)",
        background: "hsla(var(--background), 1)",
        foreground: "hsla(var(--foreground), 1)",
        primary: {
          DEFAULT: "hsla(var(--primary), 1)",
          foreground: "hsla(var(--primary-foreground), 1)"
        },
        secondary: {
          DEFAULT: "hsla(var(--secondary), 1)",
          foreground: "hsla(var(--secondary-foreground), 1)"
        },
        destructive: {
          DEFAULT: "hsla(var(--destructive), 1)",
          foreground: "hsla(var(--destructive-foreground), 1)"
        },
        muted: {
          DEFAULT: "hsla(var(--muted), 1)",
          foreground: "hsla(var(--muted-foreground), 1)"
        },
        accent: {
          DEFAULT: "hsla(var(--accent), 1)",
          foreground: "hsla(var(--accent-foreground), 1)"
        },
        popover: {
          DEFAULT: "hsla(var(--popover), 1)",
          foreground: "hsla(var(--popover-foreground), 1)"
        },
        card: {
          DEFAULT: "hsla(var(--card), 1)",
          foreground: "hsla(var(--card-foreground), 1)"
        }
      },
      borderRadius: {
        lg: `var(--radius)`,
        md: `calc(var(--radius) - 2px)`,
        sm: "calc(var(--radius) - 4px)"
      },
      keyframes: {
        "accordion-down": {
          from: { height: 0 },
          to: { height: "var(--radix-accordion-content-height)" }
        },
        "accordion-up": {
          from: { height: "var(--radix-accordion-content-height)" },
          to: { height: 0 }
        }
      },
      animation: {
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out"
      },
      typography: {
        DEFAULT: {
          css: {
            color: "hsla(var(--foreground), 1)",
            a: {
              color: "hsla(var(--primary), 1)"
            },
            h1: {
              color: "inherit",
              fontWeight: "bold"
            },
            h2: {
              color: "inherit",
              fontWeight: "bold"
            },
            h3: {
              color: "inherit",
              fontWeight: "bold"
            },
            h4: {
              color: "inherit",
              fontWeight: "bold"
            },
            h5: {
              color: "inherit",
              fontWeight: "bold"
            },
            h6: {
              color: "inherit",
              fontWeight: "bold"
            },
            strong: {
              color: "inherit"
            },
            b: {
              color: "inherit"
            },
            blockquote: {
              color: "inherit"
            },
            code: {
              color: "inherit"
            },
            img: {
              marginLeft: "auto",
              marginRight: "auto"
            }
          }
        }
      }
    }
  }
};
