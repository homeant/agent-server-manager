import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const expected = (process.env.GITHUB_REF_NAME ?? "").replace(/^v/, "");
if (!expected) throw new Error("GITHUB_REF_NAME is missing");

const rootPackage = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const cargo = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/^version = "([^"]+)"$/m)?.[1];
const desktopCargo = fs.readFileSync(path.join(root, "src-tauri", "Cargo.toml"), "utf8");
const desktopCargoVersion = desktopCargo.match(/^version = "([^"]+)"$/m)?.[1];
const desktopConfig = JSON.parse(
  fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8")
).version;
const desktopPackage = JSON.parse(
  fs.readFileSync(path.join(root, "desktop", "package.json"), "utf8")
).version;
const packages = fs
  .readdirSync(path.join(root, "npm"), { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && entry.name.startsWith("asvc-"))
  .map((entry) => path.join(root, "npm", entry.name, "package.json"));

for (const [name, version] of [
  ["package.json", rootPackage.version],
  ["Cargo.toml", cargoVersion],
  ["src-tauri/Cargo.toml", desktopCargoVersion],
  ["src-tauri/tauri.conf.json", desktopConfig],
  ["desktop/package.json", desktopPackage],
  ...packages.map((file) => [path.relative(root, file), JSON.parse(fs.readFileSync(file)).version]),
]) {
  if (version !== expected) throw new Error(`${name} is ${version}; tag is ${expected}`);
}

console.log(`release versions match ${expected}`);
