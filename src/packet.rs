use bytes::BytesMut;

pub const DUZ_UA: &str = "0";
pub const DUZ_SP: &str = "eyJvcyI6IiIsImJyb3dzZXIiOiIiLCJicm93c2VyX3VzZXJfYWdlbnQiOiIifQ==";
pub const CANARY_HOST: &str = "canary.discord.com";

/// `{"code":"vanity"}` — düzsniper byte layout
pub fn build_vanity_body(vanity: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(11 + vanity.len());
    out.extend_from_slice(b"{\"code\":\"");
    out.extend_from_slice(vanity.as_bytes());
    out.extend_from_slice(b"\"}");
    out
}

pub struct DuzTlsBase {
    pub prefix: Vec<u8>,
}

/// MFA+gid sabit kısım — Content-Length: satırına kadar (düzsniper bld)
pub fn build_duz_tls_base(
    host: &str,
    gid: &str,
    token: &str,
    mfa: &str,
    cookie: Option<&str>,
) -> DuzTlsBase {
    let cookie_line = cookie
        .filter(|c| !c.is_empty())
        .map(|c| format!("Cookie: {c}\r\n"))
        .unwrap_or_default();
    let prefix = format!(
        "PATCH /api/v9/guilds/{gid}/vanity-url HTTP/1.1\r\n\
         Host: {host}\r\n\
         Authorization: {token}\r\n\
         Content-Type: application/json\r\n\
         User-Agent: {DUZ_UA}\r\n\
         X-Super-Properties: {DUZ_SP}\r\n\
         X-Discord-MFA-Authorization: {mfa}\r\n\
         {cookie_line}\
         Connection: keep-alive\r\n\
         Content-Length: "
    );
    DuzTlsBase { prefix: prefix.into_bytes() }
}

pub fn compose_duz_tls_packet(base: &DuzTlsBase, vanity: &str) -> Vec<u8> {
    let body = build_vanity_body(vanity);
    let mut pkt = BytesMut::with_capacity(base.prefix.len() + 16 + body.len());
    pkt.extend_from_slice(&base.prefix);
    pkt.extend_from_slice(body.len().to_string().as_bytes());
    pkt.extend_from_slice(b"\r\n\r\n");
    pkt.extend_from_slice(&body);
    pkt.to_vec()
}

fn mask_secret(s: &str) -> String {
    if s.len() <= 12 {
        return "***".into();
    }
    format!("{}…{}", &s[..8], &s[s.len() - 4..])
}

/// Probe log — tam header yapısı (token/mfa kısaltılmış)
pub fn describe_tls_request(host: &str, gid: &str, token: &str, mfa: &str, vanity: &str) -> String {
    format!(
        "PATCH /api/v9/guilds/{gid}/vanity-url HTTP/1.1\n\
         Host: {host}\n\
         Authorization: {}\n\
         Content-Type: application/json\n\
         User-Agent: {DUZ_UA}\n\
         X-Super-Properties: {DUZ_SP}\n\
         X-Discord-MFA-Authorization: {}\n\
         Body: {{\"code\":\"{vanity}\"}}",
        mask_secret(token),
        mask_secret(mfa),
    )
}

pub fn describe_h2_headers(host: &str, gid: &str, token: &str, mfa: &str) -> String {
    format!(
        ":method: PATCH\n\
         :path: /api/v9/guilds/{gid}/vanity-url\n\
         :authority: {host}\n\
         authorization: {}\n\
         content-type: application/json\n\
         user-agent: {DUZ_UA}\n\
         x-super-properties: {DUZ_SP}\n\
         x-discord-mfa-authorization: {}",
        mask_secret(token),
        mask_secret(mfa),
    )
}
