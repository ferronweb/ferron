import { Plausible } from "plausible-tracker";

declare global {
  var plausible: Plausible;
}

declare module "asciinema-player";
