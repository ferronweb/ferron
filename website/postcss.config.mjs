// From https://gist.github.com/alexanderbuhler/2386befd7b6b3be3695667cb5cb5e709 and transformed from CommonJS to ES modules

import postcssOklabFunction from "@csstools/postcss-oklab-function";
import autoprefixer from "autoprefixer";
import postcss from "postcss";
import postcssMediaMinmax from "postcss-media-minmax";
import postcssNesting from "postcss-nesting";
import valueParser from "postcss-value-parser";

const propertyInjectPlugin = () => {
  return {
    Once(root) {
      const fallbackRules = [];

      // 1. Collect initial-value props from @property at-rules
      root.walkAtRules("property", (rule) => {
        const declarations = {};
        let varName = null;

        rule.walkDecls((decl) => {
          if (decl.prop === "initial-value") {
            varName = rule.params.trim();
            declarations[varName] = decl.value;
          }
        });

        if (varName) {
          fallbackRules.push(`${varName}: ${declarations[varName]};`);
        }
      });

      // 2. Inject fallback variables if any exist
      if (fallbackRules.length > 0) {
        // check for paint() because its browser support aligns with @property at-rule
        const fallbackCSS = `@supports not (background: paint(something)) {
                    :root { ${fallbackRules.join(" ")} }
                }`;

        const sourceFile = root.source?.input?.file || root.source?.input?.from;
        const fallbackAst = postcss.parse(fallbackCSS, { from: sourceFile });

        // Insert after last @import (or prepend if none found)
        let lastImportIndex = -1;
        root.nodes.forEach((node, i) => {
          if (node.type === "atrule" && node.name === "import") {
            lastImportIndex = i;
          }
        });

        if (lastImportIndex === -1) {
          root.prepend(fallbackAst);
        } else {
          root.insertAfter(root.nodes[lastImportIndex], fallbackAst);
        }
      }

      // 3. Remove `in <colorspace>` after `to left` or `to right`, e.g. "to right in oklab" -> "to right"
      root.walkDecls((decl) => {
        if (!decl.value) return;

        decl.value = decl.value.replaceAll(
          /\bto\s+(left|right)\s+in\s+[\w-]+/g,
          (_, direction) => {
            return `to ${direction}`;
          }
        );
      });
    },
    postcssPlugin: "postcss-property-polyfill"
  };
};

propertyInjectPlugin.postcss = true;

/*
    This plugin resolves/calculates CSS variables within color-mix() functions so they can be calculated using postcss-color-mix-function
    Exception: dynamic values like currentColor
*/
const colorMixVarResolverPlugin = () => {
  return {
    Once(root) {
      const cssVariables = {};

      // 1. Collect all CSS variable definitions from tailwind
      root.walkRules((rule) => {
        if (!rule.selectors) return;

        const isRootOrHost = rule.selectors.some(
          (sel) => sel.includes(":root") || sel.includes(":host")
        );

        if (isRootOrHost) {
          // Collect all --var declarations in this rule
          rule.walkDecls((decl) => {
            if (decl.prop.startsWith("--")) {
              cssVariables[decl.prop] = decl.value.trim();
            }
          });
        }
      });

      // 2. Parse each declaration's value and replace var(...) in color-mix(...)
      root.walkDecls((decl) => {
        const originalValue = decl.value;
        if (!originalValue || !originalValue.includes("color-mix(")) return;

        const parsed = valueParser(originalValue);
        let modified = false;

        parsed.walk((node) => {
          if (node.type === "function" && node.value === "color-mix") {
            node.nodes.forEach((childNode) => {
              if (
                childNode.type === "function" &&
                childNode.value === "var" &&
                childNode.nodes.length > 0
              ) {
                const varName = childNode.nodes[0]?.value;
                if (!varName) return;

                const resolvedVarName =
                  cssVariables[varName] === undefined
                    ? "black"
                    : cssVariables[varName]; // fall back to black if var is undefined
                // add whitespace because it might just be a part of a color notation e.g. #fff 10%
                const resolved = resolvedVarName
                  ? `${resolvedVarName} `
                  : `var(${varName})`;

                childNode.type = "word";
                childNode.value = resolved;
                childNode.nodes = [];
                modified = true;
              }
            });
          }
        });

        if (modified) {
          const newValue = parsed.toString();
          decl.value = newValue;
        }
      });
    },

    postcssPlugin: "postcss-color-mix-var-resolver"
  };
};

colorMixVarResolverPlugin.postcss = true;

/*
    This plugin transforms shorthand rotate/scale/translate into their transform[3d] counterparts
*/
const transformShortcutPlugin = () => {
  return {
    Once(root) {
      const defaults = {
        rotate: [0, 0, 1, "0deg"],
        scale: [1, 1, 1],
        translate: [0, 0, 0]
      };

      const fallbackAtRule = postcss.atRule({
        name: "supports",
        params: "not (translate: 0)" // or e.g. 'not (translate: 1px)'
      });

      root.walkRules((rule) => {
        let hasTransformShorthand = false;
        const transformFunctions = [];

        rule.walkDecls((decl) => {
          if (/^(rotate|scale|translate)$/.test(decl.prop)) {
            hasTransformShorthand = true;

            const newValues = [...defaults[decl.prop]];
            // add whitespaces for minified vars
            const value = decl.value.replaceAll(/\)\s*var\(/g, ") var(");
            const userValues = postcss.list.space(value);

            // special case: rotate w/ single angle only
            if (decl.prop === "rotate" && userValues.length === 1) {
              newValues.splice(-1, 1, ...userValues);
            } else {
              // for scale/translate, or rotate with multiple params
              newValues.splice(0, userValues.length, ...userValues);
            }

            // e.g. "translate3d(10px,20px,0)"
            transformFunctions.push(`${decl.prop}3d(${newValues.join(",")})`);
          }
        });

        // Process rotate/scale/translate in this rule:
        if (hasTransformShorthand && transformFunctions.length > 0) {
          const fallbackRule = postcss.rule({ selector: rule.selector });

          fallbackRule.append({
            prop: "transform",
            value: transformFunctions.join(" ")
          });

          fallbackAtRule.append(fallbackRule);
        }
      });

      if (fallbackAtRule.nodes && fallbackAtRule.nodes.length > 0) {
        root.append(fallbackAtRule);
      }
    },

    postcssPlugin: "postcss-transform-shortcut"
  };
};

transformShortcutPlugin.postcss = true;

/**
 * PostCSS plugin to transform empty fallback values from `var(--foo,)`,
 * turning them into `var(--foo, )`. Older browsers need this.
 */
const addSpaceForEmptyVarFallback = () => {
  return {
    /**
     * We do our edits in `OnceExit`, meaning we process each decl after
     * the AST is fully built and won't get re-visited or re-triggered
     * in the same pass.
     */
    OnceExit(root) {
      root.walkDecls((decl) => {
        if (!decl.value || !decl.value.includes("var(")) {
          return;
        }

        const parsed = valueParser(decl.value);
        let changed = false;

        parsed.walk((node) => {
          // Only consider var(...) function calls
          if (node.type === "function" && node.value === "var") {
            // Look for the `div` node with value "," that separates property & fallback
            const commaIndex = node.nodes.findIndex(
              (n) => n.type === "div" && n.value === ","
            );

            // If no comma is found, no fallback segment
            if (commaIndex === -1) return;

            // Gather any fallback text
            const fallbackNodes = node.nodes.slice(commaIndex + 1);
            const fallbackText = fallbackNodes
              .map((n) => n.value)
              .join("")
              .trim();

            // If there's no fallback text => `var(--something,)` => we insert a space
            if (fallbackText === "") {
              const commaNode = node.nodes[commaIndex];
              // If the comma node is literally "," with no space, change it to ", "
              if (commaNode.value === ",") {
                commaNode.value = ", ";
                changed = true;
              }
            }
          }
        });

        if (changed) {
          decl.value = parsed.toString();
        }
      });
    },

    postcssPlugin: "postcss-add-space-for-empty-var-fallback"
  };
};

addSpaceForEmptyVarFallback.postcss = true;

const config = {
  plugins: [
    transformShortcutPlugin(),
    addSpaceForEmptyVarFallback(),
    postcssMediaMinmax,
    postcssOklabFunction,
    postcssNesting,
    autoprefixer
  ]
};

export default config;
