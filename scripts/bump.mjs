// 版本號 +0.0.1：同步改 src-tauri/tauri.conf.json、src-tauri/Cargo.toml、package.json
// 用法：node scripts/bump.mjs
import fs from "node:fs";
const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
const [a, b, c] = conf.version.split(".").map(Number);
const next = `${a}.${b}.${c + 1}`;
const replace = (file, re, to) => fs.writeFileSync(file, fs.readFileSync(file, "utf8").replace(re, to));
replace("src-tauri/tauri.conf.json", /"version":\s*"[^"]+"/, `"version": "${next}"`);
replace("package.json", /"version":\s*"[^"]+"/, `"version": "${next}"`);
replace("src-tauri/Cargo.toml", /^version\s*=\s*"[^"]+"/m, `version = "${next}"`);
console.log(`${conf.version} → ${next}`);
