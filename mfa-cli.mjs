#!/usr/bin/env node
/** Rust kod5 → Node MFA bridge (stdout = {"token","cookie"}) */
import { get } from "./mfa.mjs";

const token = process.env.MFA_TOKEN || process.argv[2];
const password = process.env.MFA_PASSWORD || process.argv[3];
const gid = process.env.MFA_GID || process.argv[4] || "1";

if (!token || !password) {
  console.error("MFA_TOKEN + MFA_PASSWORD gerekli");
  process.exit(1);
}

process.env.MFA_GID = String(gid).replace(/\D/g, "") || "1";

try {
  const out = await get(token, password);
  const jwt = typeof out === "string" ? out : out?.token;
  const cookie = typeof out === "object" && out?.cookie ? out.cookie : "";
  if (!jwt) {
    console.error("MFA token bos");
    process.exit(1);
  }
  process.stdout.write(JSON.stringify({ token: jwt, cookie }));
} catch (e) {
  console.error(e?.message || String(e));
  process.exit(1);
}
