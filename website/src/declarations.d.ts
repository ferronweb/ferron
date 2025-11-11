import { Plausible } from "@plausible-analytics/tracker";

declare global {
  var plausible: Plausible;
}

declare module "asciinema-player";
