"use strict";
import tls from "node:tls";
import { randomUUID } from "node:crypto";

const CL = (...a) => { if (process.env.MFA_DEBUG === "1") console.error("[rush-mfa]", ...a); };
const H = "discord.com";
const BUILD = 565311, NATIVE = 84410, VER = "1.0.9242", EV = "37.6.0", CV = "138.0.7204.251";
const UA = `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/${VER} Chrome/${CV} Electron/${EV} Safari/537.36`;
const _CKN = ["__dcfduid", "__sdcfduid", "__stripe_mid", "cf_clearance", "_cfuvid", "__Secure-recent_mfa"];
const deadTokens = new Set();
const sl = ms => new Promise(r => setTimeout(r, ms));

const ctx = tls.createSecureContext({
  minVersion: "TLSv1.2", maxVersion: "TLSv1.2", ecdhCurve: "X25519:P-256:P-384", honorCipherOrder: true,
  ciphers: "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305"
});

const TICKET_BODY = Buffer.from('{"code":"000000","secret":"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"}');
const VANITY_BODY = Buffer.from("{}");

const ME_PATH = "/api/v9/users/@me";
const TOTP_PATH = "/api/v9/users/@me/mfa/totp/enable";
const FINISH_PATH = "/api/v9/mfa/finish";

const vanityPath = () => {
  const gid = String(process.env.MFA_GID || "1").replace(/\D/g, "") || "1";
  return `/api/v9/guilds/${gid}/vanity-url`;
};

let spB64 = null;
let ck = "", _ckd = false;
let useTotp = true;

const initHeaders = () => {
  if (spB64) return;
  spB64 = Buffer.from(JSON.stringify({
    os: "Windows", browser: "Discord Client", release_channel: "stable", client_version: VER, os_version: "10.0.19045", os_arch: "x64", app_arch: "x64", system_locale: "tr", has_client_mods: false, client_launch_id: randomUUID(), browser_user_agent: UA, browser_version: EV, os_sdk_version: "19045", client_build_number: BUILD, native_build_number: NATIVE, client_event_source: null, launch_signature: randomUUID(), client_heartbeat_session_id: randomUUID(), client_app_state: "focused"
  })).toString("base64");
};

const _pSC = (hdrs) => {
  try {
    const sc = hdrs["set-cookie"]; if (!sc) return;
    const arr = Array.isArray(sc) ? sc : String(sc).split(/,(?=\s*\w+=)/);
    const co = {};
    for (const s of arr) { const nv = s.split(";")[0]; const ei = nv.indexOf("="); if (ei < 1) continue; const n = nv.slice(0, ei).trim(), v = nv.slice(ei + 1).trim(); if (n && v && _CKN.includes(n)) co[n] = v; }
    if (Object.keys(co).length) {
      const old = {};
      if (ck) for (const p of ck.split("; ")) { const i = p.indexOf("="); if (i > 0) old[p.slice(0, i)] = p.slice(i + 1); }
      Object.assign(old, co);
      const parts = []; for (const k of _CKN) { if (old[k]) parts.push(`${k}=${old[k]}`); }
      ck = parts.join("; ");
    }
  } catch {}
};

const parseRespHeaders = (raw) => {
  const lines = raw.split("\r\n");
  const status = parseInt(lines[0].split(" ")[1], 10) || 0;
  const headers = {};
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i]; if (!line) continue;
    const ci = line.indexOf(":"); if (ci < 0) continue;
    const key = line.slice(0, ci).trim().toLowerCase();
    const val = line.slice(ci + 1).trim();
    if (headers[key]) {
      headers[key] = Array.isArray(headers[key]) ? [...headers[key], val] : [headers[key], val];
    } else headers[key] = val;
  }
  return { status, headers };
};

const decodeChunked = (buf) => {
  const chunks = [];
  let pos = 0;
  while (pos < buf.length) {
    const lineEnd = buf.indexOf("\r\n", pos);
    if (lineEnd === -1) break;
    const size = parseInt(buf.subarray(pos, lineEnd).toString("utf-8").split(";")[0].trim(), 16);
    if (Number.isNaN(size)) break;
    pos = lineEnd + 2;
    if (size === 0) break;
    chunks.push(buf.subarray(pos, pos + size));
    pos += size + 2;
  }
  return Buffer.concat(chunks).toString("utf-8");
};

const readBody = (bodyPart, headers) => {
  const te = String(headers["transfer-encoding"] || "").toLowerCase();
  if (te.includes("chunked")) return decodeChunked(bodyPart);
  const head = bodyPart.subarray(0, Math.min(12, bodyPart.length)).toString("utf-8");
  if (/^[0-9a-fA-F]+\r\n/.test(head)) return decodeChunked(bodyPart);
  const cl = headers["content-length"];
  if (cl !== undefined) {
    const need = parseInt(cl, 10);
    return bodyPart.subarray(0, Number.isNaN(need) ? bodyPart.length : need).toString("utf-8");
  }
  return bodyPart.toString("utf-8");
};

const tlsReq = (method, path, tk, bodyBuf) => new Promise((resolve, reject) => {
  initHeaders();
  let hdr = `${method} ${path} HTTP/1.1\r\nHost: ${H}\r\nAuthorization: ${tk}\r\nContent-Type: application/json\r\nUser-Agent: ${UA}\r\nX-Super-Properties: ${spB64}\r\nConnection: close\r\n`;
  if (ck) hdr += `Cookie: ${ck}\r\n`;
  if (bodyBuf) hdr += `Content-Length: ${bodyBuf.length}\r\n`;
  hdr += "\r\n";
  const reqBuf = bodyBuf ? Buffer.concat([Buffer.from(hdr), bodyBuf]) : Buffer.from(hdr);

  const sock = tls.connect({
    host: H, port: 443, servername: H, secureContext: ctx,
    rejectUnauthorized: false, ALPNProtocols: ["http/1.1"], highWaterMark: 0
  });
  sock.setNoDelay(true);
  let buf = Buffer.alloc(0), done = false;
  const fin = (fn, v) => { if (done) return; done = true; sock.removeAllListeners(); sock.setTimeout(0); try { sock.destroy(); } catch {} fn(v); };

  sock.once("secureConnect", () => sock.write(reqBuf));
  sock.on("data", c => { buf = buf.length ? Buffer.concat([buf, c]) : c; });
  sock.once("end", () => {
    try {
      const sep = buf.indexOf("\r\n\r\n");
      if (sep === -1) return fin(reject, new Error("bad_response"));
      const { status, headers } = parseRespHeaders(buf.subarray(0, sep).toString("utf-8"));
      const data = readBody(buf.subarray(sep + 4), headers);
      fin(resolve, { status, headers, data });
    } catch (e) { fin(reject, e); }
  });
  sock.once("error", e => fin(reject, e));
  sock.setTimeout(15000, () => fin(reject, new Error("timeout")));
});

const U401_DELAYS = [1000, 3000, 5000];

const is401R = r => (r?._st === 401 && r?.code !== 60003) || r?.message === "401: Unauthorized";
const hasTicket = r => !!(r?.mfa?.ticket || r?.ticket);

const ticketPaths = totpFirst => totpFirst
  ? [
      { label: "ticket_totp", path: TOTP_PATH, method: "POST", body: TICKET_BODY },
      { label: "ticket_vanity", path: vanityPath(), method: "PATCH", body: VANITY_BODY },
    ]
  : [
      { label: "ticket_vanity", path: vanityPath(), method: "PATCH", body: VANITY_BODY },
      { label: "ticket_totp", path: TOTP_PATH, method: "POST", body: TICKET_BODY },
    ];

const fetchTicket = async (token, totpFirst) => {
  for (const p of ticketPaths(totpFirst)) {
    const tr = await rq401(p.label, () => rq(p.path, p.method, p.body, token));
    if (tr?._st === 200 && !hasTicket(tr)) {
      CL(`${p.label} 200 (ticket yok) -> diger path`);
      continue;
    }
    return tr;
  }
  return { _st: 200, _no_ticket: 1 };
};

const rq401 = async (label, fn) => {
  let r;
  for (let i = 0; i < 3; i++) {
    r = await fn();
    if (!is401R(r)) return r;
    if (i === 2) return r;
    CL(`${label} 401 retry ${i + 1}/3 in ${U401_DELAYS[i]}ms`);
    await sl(U401_DELAYS[i]);
  }
  return r;
};

const rq = async (p, m, bdBuf, tk, rt = 0) => {
  try {
    const resp = await tlsReq(m, p, tk, bdBuf);
    _pSC(resp.headers || {});
    const st = resp.status;
    const rawData = String(resp.data || "").trim();
    let j;
    try {
      j = rawData ? JSON.parse(rawData) : {};
    } catch {
      const raw = rawData.slice(0, 300);
      if (st === 429 || raw.includes("rate") || raw.includes("cloudflare") || raw.includes("1015")) { const rm = raw.match(/retry[_-]?after[":\s]+(\d+\.?\d*)/i); const ra = rm ? parseFloat(rm[1]) : 10; return { _rl: 1, _ra: ra, _cd: Math.ceil(ra), _st: st }; }
      if (rt < 3) { CL(`rq parse fail rt=${rt} st=${st} raw=${JSON.stringify(raw)}`); await sl(500); return rq(p, m, bdBuf, tk, rt + 1); }
      return { _err: 1, _st: st, _r: raw };
    }
    j._st = st;
    if (j.code === 1015 || (j.message && String(j.message).includes("1015"))) { const ra = j.retry_after || 10; return { _rl: 1, _ra: ra, _cd: Math.ceil(ra), _st: st }; }
    if (st === 429) { const ra = j.retry_after || 5; j._rl = 1; j._ra = ra; j._cd = Math.ceil(ra); }
    if (st === 403 && rt < 3) { CL(`rq 403 rt=${rt}`); return rq(p, m, bdBuf, tk, rt + 1); }
    return j;
  } catch (e) {
    if (rt < 3) { CL(`rq err rt=${rt}: ${e.message}`); await sl(500); return rq(p, m, bdBuf, tk, rt + 1); }
    throw e;
  }
};

const _fCk = async (tk) => { try { await rq(ME_PATH, "GET", null, tk); _ckd = true; } catch { _ckd = true; } };

const get = (token, password, optsOrCb) => {
  let opts = {}, cb; if (typeof optsOrCb === "function") { cb = optsOrCb; } else if (typeof optsOrCb === "object" && optsOrCb !== null) { opts = optsOrCb; }
  const _retry = opts._retry || 0;
  const log = (...a) => CL(...a);

  const pr = (async () => {
    if (_retry >= 3) throw new Error("MAX_RETRY_EXCEEDED");
    if (deadTokens.has(token)) throw new Error("TOKEN_DEAD");
    if (!_ckd) await _fCk(token);
    let tk = null;
    let ti = 0;

    while (!tk) {
      if (ti >= 3) throw new Error("NO_TICKET_MAX_RETRY");
      if (ti > 0) await sl(2000);

      let tr;
      try {
        tr = await fetchTicket(token, useTotp);
        useTotp = !useTotp;
      } catch (e) {
        log("ticket err:", e.message); ti++; continue;
      }

      if (tr?._no_ticket) { log("ticket 200 both paths, retry"); ti++; continue; }

      if (is401R(tr)) {
        log("ticket 401 -> token dead");
        deadTokens.add(token); throw new Error("TOKEN_DEAD_401");
      }

      if (tr?._err) { log("ticket _err:", JSON.stringify(tr)); ti++; continue; }
      if (tr?._rl) { ti++; await sl(tr._ra ? tr._ra * 1000 : 2000); continue; }
      if (tr?.code === 0) { deadTokens.add(token); throw new Error("TOKEN_DEAD"); }

      tk = tr?.mfa?.ticket || tr?.ticket;
      if (!tk) { log("no ticket in response:", JSON.stringify(tr)); ti++; }
    }

    const FINISH_BODY = Buffer.from(JSON.stringify({ ticket: tk, mfa_type: "password", data: password }));
    let r;
    try {
      r = await rq401("finish", () => rq(FINISH_PATH, "POST", FINISH_BODY, token));
    } catch (e) {
      log("finish err:", e.message); await sl(2000);
      return get(token, password, { ...opts, _retry: _retry + 1 });
    }

    if (is401R(r)) {
      log("finish 401 -> token dead");
      deadTokens.add(token); throw new Error("TOKEN_DEAD_401");
    }

    if (r?._err) {
      log("finish _err:", JSON.stringify(r));
      await sl(2000);
      return get(token, password, { ...opts, _retry: _retry + 1 });
    }

    if (r?.code === 60008 || String(r?.message).includes("password")) { deadTokens.add(token); throw new Error("60008_INVALID_PASSWORD"); }
    if (r?.token) return { token: r.token, cookie: ck };

    log("retry rt=" + _retry + ":", r?.message || r?.code || "no token");
    await sl(2000);
    return get(token, password, { ...opts, _retry: _retry + 1 });
  })();

  if (typeof cb === "function") { pr.then(t => cb(null, t)).catch(e => cb(e, null)); return; }
  return pr;
};

export { get };
export default { get };
