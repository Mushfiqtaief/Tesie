use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::{json, Map, Value};
use tesie_core::{ProxyNode, Result, TesRouteError};
use url::Url;

pub const TESROUTE_POOL_GROUP: &str = "TESIE_POOL";
pub const DEFAULT_MIHOMO_TEST_URL: &str = "https://www.gstatic.com/generate_204";

pub fn validate_proxy_url(proxy_url: &str) -> Result<()> {
    parse_proxy_url_to_mihomo("_check", proxy_url)?;
    Ok(())
}

pub fn parse_proxy_url_to_mihomo(name: &str, proxy_url: &str) -> Result<Value> {
    let trimmed = proxy_url.trim();
    if trimmed.is_empty() {
        return Err(TesRouteError::BadRequest("proxy_url is empty".into()));
    }

    if trimmed.to_ascii_lowercase().starts_with("ss://") {
        return parse_ss_url(name, trimmed);
    }

    let parsed = Url::parse(trimmed)
        .map_err(|e| TesRouteError::BadRequest(format!("invalid proxy URL {trimmed}: {e}")))?;

    let scheme = parsed.scheme().to_ascii_lowercase();
    let host = parsed
        .host_str()
        .ok_or_else(|| TesRouteError::BadRequest(format!("proxy URL missing host: {trimmed}")))?;

    let mut obj = Map::new();
    obj.insert("name".into(), Value::String(name.to_string()));
    obj.insert("server".into(), Value::String(host.to_string()));

    match scheme.as_str() {
        "http" => {
            obj.insert("type".into(), Value::String("http".into()));
            obj.insert(
                "port".into(),
                Value::Number(parsed.port().unwrap_or(80).into()),
            );
        }
        "https" => {
            obj.insert("type".into(), Value::String("http".into()));
            obj.insert(
                "port".into(),
                Value::Number(parsed.port().unwrap_or(443).into()),
            );
            obj.insert("tls".into(), Value::Bool(true));
        }
        "socks4" => {
            obj.insert("type".into(), Value::String("socks4".into()));
            obj.insert(
                "port".into(),
                Value::Number(parsed.port().unwrap_or(1080).into()),
            );
        }
        "socks4a" => {
            obj.insert("type".into(), Value::String("socks4a".into()));
            obj.insert(
                "port".into(),
                Value::Number(parsed.port().unwrap_or(1080).into()),
            );
        }
        "socks" | "socks5" | "socks5h" => {
            obj.insert("type".into(), Value::String("socks5".into()));
            obj.insert(
                "port".into(),
                Value::Number(parsed.port().unwrap_or(1080).into()),
            );
            obj.insert("udp".into(), Value::Bool(true));
        }
        other => {
            return Err(TesRouteError::BadRequest(format!(
                "unsupported proxy scheme '{other}' (use socks4, socks4a, socks5, socks5h, http, https, ss, or raw_outbound_json)"
            )));
        }
    }

    if !parsed.username().is_empty() {
        obj.insert(
            "username".into(),
            Value::String(percent_decode(parsed.username())),
        );
    }
    if let Some(pass) = parsed.password() {
        obj.insert("password".into(), Value::String(percent_decode(pass)));
    }

    Ok(Value::Object(obj))
}

fn parse_ss_url(name: &str, raw: &str) -> Result<Value> {
    let without_scheme = &raw[5..];
    let without_frag = without_scheme.split('#').next().unwrap_or(without_scheme);
    let body = without_frag.split('?').next().unwrap_or(without_frag);

    let (userinfo_raw, host_port) = if let Some((left, right)) = body.rsplit_once('@') {
        (left, right)
    } else {
        let decoded = decode_base64_flexible(body)?;
        let (left, right) = decoded
            .rsplit_once('@')
            .ok_or_else(|| TesRouteError::BadRequest("invalid ss:// URL".into()))?;
        return build_ss_object(name, left, right);
    };

    let userinfo = if userinfo_raw.contains(':') {
        userinfo_raw.to_string()
    } else {
        decode_base64_flexible(userinfo_raw)?
    };

    build_ss_object(name, &userinfo, host_port)
}

fn build_ss_object(name: &str, userinfo: &str, host_port: &str) -> Result<Value> {
    let (cipher, password) = userinfo
        .split_once(':')
        .ok_or_else(|| TesRouteError::BadRequest("ss:// missing cipher:password".into()))?;

    let dummy = Url::parse(&format!("http://{host_port}"))
        .map_err(|e| TesRouteError::BadRequest(format!("invalid ss:// host:port: {e}")))?;
    let server = dummy
        .host_str()
        .ok_or_else(|| TesRouteError::BadRequest("ss:// missing server host".into()))?;
    let port = dummy
        .port()
        .ok_or_else(|| TesRouteError::BadRequest("ss:// missing server port".into()))?;

    Ok(json!({
        "name": name,
        "type": "ss",
        "server": server,
        "port": port,
        "cipher": cipher,
        "password": password,
        "udp": true
    }))
}

fn decode_base64_flexible(input: &str) -> Result<String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| URL_SAFE.decode(input))
        .or_else(|_| STANDARD.decode(input))
        .map_err(|e| TesRouteError::BadRequest(format!("invalid base64 in ss:// URL: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|e| TesRouteError::BadRequest(format!("invalid utf8 in ss:// URL: {e}")))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            out.push(((hi << 4) | lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

pub fn node_to_mihomo_outbound(node: &ProxyNode) -> Result<Option<Value>> {
    if node.is_direct || !node.enabled {
        return Ok(None);
    }

    if let Some(raw_json) = node.raw_outbound_json.as_deref().map(str::trim)
        && !raw_json.is_empty()
    {
        let mut val: Value = serde_json::from_str(raw_json).map_err(|e| {
            TesRouteError::BadRequest(format!(
                "invalid raw_outbound_json for {}: {e}",
                node.name
            ))
        })?;
        let obj = val.as_object_mut().ok_or_else(|| {
            TesRouteError::BadRequest(format!(
                "raw_outbound_json for {} must be a JSON object",
                node.name
            ))
        })?;
        obj.insert("name".into(), Value::String(node.name.clone()));
        return Ok(Some(val));
    }

    if let Some(proxy_url) = node.proxy_url.as_deref().map(str::trim)
        && !proxy_url.is_empty()
    {
        return Ok(Some(parse_proxy_url_to_mihomo(&node.name, proxy_url)?));
    }

    Ok(None)
}

pub fn generate_mihomo_yaml(
    ctl_addr: &str,
    ctl_secret: &str,
    nodes: &[ProxyNode],
) -> Result<String> {
    let mut proxies = Vec::new();
    let mut pool_member_names = vec!["DIRECT".to_string()];

    for node in nodes {
        match node_to_mihomo_outbound(node) {
            Ok(Some(outbound)) => {
                pool_member_names.push(node.name.clone());
                proxies.push(outbound);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(proxy = %node.name, error = %err, "skipping invalid proxy node in mihomo config");
            }
        }
    }

    let proxy_groups = vec![json!({
        "name": TESROUTE_POOL_GROUP,
        "type": "select",
        "url": DEFAULT_MIHOMO_TEST_URL,
        "proxies": pool_member_names,
    })];

    let doc = json!({
        "mode": "rule",
        "ipv6": false,
        "allow-lan": false,
        // 0 disables the inbound listener: the relay only dials out via /tesie/relay.
        // Unset, mihomo binds its default 7890 and collides with other mihomo processes.
        "mixed-port": 0,
        "port": 0,
        "socks-port": 0,
        "log-level": "warning",
        "external-controller": ctl_addr,
        "secret": ctl_secret,
        "dns": {
            "enable": false,
            "default-nameserver": ["system", "1.1.1.1", "8.8.8.8"]
        },
        "proxies": proxies,
        "proxy-groups": proxy_groups,
        "rules": ["MATCH,DIRECT"]
    });

    serde_yaml::to_string(&doc)
        .map_err(|e| TesRouteError::Internal(format!("serialize mihomo yaml: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_node(name: &str, url: Option<&str>, raw: Option<&str>, is_direct: bool) -> ProxyNode {
        ProxyNode {
            id: name.into(),
            name: name.into(),
            proxy_type: if is_direct { "direct".into() } else { "http".into() },
            ip: None,
            host: None,
            port: None,
            username: None,
            password: None,
            latency_ms: None,
            alive: true,
            proxy_url: url.map(str::to_string),
            raw_outbound_json: raw.map(str::to_string),
            is_direct,
            enabled: true,
            egress_ip: None,
            last_delay_ms: None,
            last_probed_at: None,
            consecutive_failures: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn parses_proxy_urls() {
        let http = parse_proxy_url_to_mihomo("us-1", "http://alice:s3cr3t@1.2.3.4:8080").unwrap();
        assert_eq!(http["type"], "http");
        assert_eq!(http["port"], 8080);
        assert_eq!(http["username"], "alice");
        assert_eq!(http["password"], "s3cr3t");

        let socks4 = parse_proxy_url_to_mihomo("s4", "socks4://103.17.37.196:33801").unwrap();
        assert_eq!(socks4["type"], "socks4");
        assert_eq!(socks4["port"], 33801);

        let socks = parse_proxy_url_to_mihomo("s5", "socks5://127.0.0.1:1080").unwrap();
        assert_eq!(socks["type"], "socks5");
        assert_eq!(socks["udp"], true);

        let socks_bare = parse_proxy_url_to_mihomo("s0", "socks://127.0.0.1:1080").unwrap();
        assert_eq!(socks_bare["type"], "socks5");
        assert_eq!(socks_bare["port"], 1080);

        let socks5h_cred =
            parse_proxy_url_to_mihomo("s5h", "socks5h://user%40name:a+b%3Dc@9.8.7.6:1081").unwrap();
        assert_eq!(socks5h_cred["type"], "socks5");
        assert_eq!(socks5h_cred["username"], "user@name");
        assert_eq!(socks5h_cred["password"], "a+b=c");

        let encoded = URL_SAFE_NO_PAD.encode("aes-256-gcm:my-pass");
        let ss =
            parse_proxy_url_to_mihomo("ss1", &format!("ss://{encoded}@5.6.7.8:8388#SG")).unwrap();
        assert_eq!(ss["type"], "ss");
        assert_eq!(ss["cipher"], "aes-256-gcm");
        assert_eq!(ss["password"], "my-pass");
    }

    #[test]
    fn generates_yaml_with_tesroute_pool_group_and_skips_direct() {
        let nodes = vec![
            mk_node("DIRECT", None, None, true),
            mk_node("p1", Some("http://127.0.0.1:8080"), None, false),
        ];
        let yaml = generate_mihomo_yaml("127.0.0.1:29090", "secret", &nodes).unwrap();
        assert!(yaml.contains("name: p1"));
        assert!(yaml.contains("TESIE_POOL"));
        assert!(!yaml.contains("name: DIRECT"));
    }
}
