//! UPnP IGD automatic port-mapping for the game port, attempted once at boot.
//!
//! The same job AstroLauncher does for its own Terraria/Minecraft/Valheim server launcher: on
//! startup, ask the router (via UPnP's SSDP discovery, then a SOAP `AddPortMapping` call) to
//! forward the configured game port to this machine, so a home operator behind NAT does not have
//! to find their router's port-forwarding page by hand. When no UPnP-capable router answers, or
//! it refuses the request (UPnP disabled, a corporate/ISP router, or simply no NAT at all), this
//! logs a clear, specific fallback message naming the port and the local address to forward it
//! to: never a fatal error, and never something a running server waits on. [`attempt`] is meant
//! to be spawned as a background task, the same way `update::boot_check` is.
//!
//! This has nothing to do with the web admin panel, which stays bound to loopback regardless of
//! anything here: see `config.rs`'s `panel_listen` doc comment.
//!
//! # Hand-rolled, and why it can be
//!
//! This module replaced the `igd-next` dependency (31 crates, `hyper`/`url`/`idna` and a
//! transitive `ring`) with the two entry points that were actually used: find a gateway, add one
//! port mapping. IGD control traffic is plain HTTP/1.1 over the LAN against raw `IP:port`
//! literals, so none of TLS, real DNS, or internationalized-domain handling applies, which is
//! what made the dependency's weight so lopsided against its job.
//!
//! Everything that decides anything is a pure function over a `&str`, unit-tested below against
//! captured output from real routers (see the fixtures' own citations). The socket I/O is a thin
//! shell around those: one UDP multicast send/receive, one HTTP GET, one HTTP POST.
//!
//! # What is *not* verified
//!
//! **The live socket path has never run against a real UPnP router in CI, and cannot.** CI has no
//! IGD to answer an M-SEARCH, so every test here exercises the parsing, not the exchange. Before
//! this is trusted in production, a human has to run `cargo run --example upnp_probe` on a real
//! home network behind a real UPnP-capable router and confirm both that the mapping appears in
//! the router's own port-forwarding table and that an outside client can then reach the server.
//! The fallback path (no router answers) is exercised for real by any sandbox, which is the only
//! half of this a runner can honestly claim.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    time::timeout,
};
use tracing::{debug, info, warn};

/// Real router firmware does not reliably honour a `0` (infinite) lease the way the UPnP spec
/// technically allows: plenty of implementations expire it anyway. Two hours, renewed at the
/// halfway point by this function's own loop, is a conservative, widely-used middle ground (the
/// same order of magnitude other UPnP port-mapping tools default to) rather than either extreme.
const LEASE_SECS: u32 = 7_200;

/// How long discovery may take in total, and how long any one datagram is waited for. Both match
/// `igd-next`'s own `SearchOptions::default()` (`DEFAULT_TIMEOUT` 10s, `RESPONSE_TIMEOUT` 5s)
/// exactly, so replacing it did not change how long a router-less boot spends finding that out.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// A bound `igd-next` did *not* have: its tokio SOAP client is a bare `hyper` client with no
/// timeout at all, so a router that accepted the connection and then went quiet would hang the
/// mapping future forever. Nothing waits on this task, so that was survivable rather than
/// harmless; ten seconds for a LAN request is generous either way.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Cap on how much a device on the LAN can make this read. Device descriptions are a few
/// kilobytes; `igd-next` capped its own SSDP reads at 1500 bytes for the same reason.
const MAX_HTTP_RESPONSE: usize = 64 * 1024;

/// The SSDP multicast group and port, fixed by the UPnP Device Architecture.
const SSDP_MULTICAST: &str = "239.255.255.250:1900";

/// What the mapping is labelled as in the router's own port-forwarding table.
const MAPPING_DESCRIPTION: &str = "terrustia";

/// Search targets to announce, in order. `igd-next` only ever asked for `InternetGatewayDevice:1`,
/// which an IGD:2-only router (the Sagemcom Livebox in this module's own test fixtures is one)
/// never answers.
const SEARCH_TARGETS: [&str; 2] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
];

/// The service types that can actually map a port. `igd-next` accepted the first three;
/// `WANPPPConnection:2` is added because a real IGD:2 device advertises it (again, the Livebox
/// fixture) and nothing about it is different for our one call.
const WAN_SERVICES: [&str; 4] = [
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:2",
];

/// A router that answered discovery and advertises a service that can map ports.
struct Gateway {
    /// Absolute `http://host:port/path` control endpoint, already resolved against `URLBase`.
    control_url: String,
    /// The exact `serviceType` string the router advertised, which the SOAP envelope and the
    /// `SOAPAction` header both have to name back at it.
    service_type: String,
}

/// Attempt UPnP port-mapping for `listen`'s port, once, then keep the lease renewed for as long
/// as the returned future runs. Returns immediately, without attempting anything, for a
/// loopback-only `listen`: there is nothing on the public internet a router mapping would help
/// reach in that case.
pub async fn attempt(listen: SocketAddr) {
    if listen.ip().is_loopback() {
        return;
    }

    let Some(local_ip) = local_ipv4() else {
        warn!(
            "could not determine this machine's local network address; skipping UPnP port \
             mapping. Forward TCP port {} manually if you want this server reachable from \
             outside your network",
            listen.port()
        );
        return;
    };
    let local_addr = SocketAddr::new(IpAddr::V4(local_ip), listen.port());

    let gateway = match discover().await {
        Ok(gateway) => gateway,
        Err(e) => {
            info!(
                error = %e,
                port = listen.port(),
                %local_addr,
                "no UPnP-capable router found (or UPnP is disabled on it). Forward TCP port {} \
                 to {local_addr} on your router manually if you want this server reachable from \
                 outside your network",
                listen.port()
            );
            return;
        }
    };

    if let Err(e) = map_once(&gateway, listen.port(), local_addr).await {
        info!(
            error = %e,
            port = listen.port(),
            %local_addr,
            "the router refused the UPnP port mapping request. Forward TCP port {} to \
             {local_addr} manually if you want this server reachable from outside your network",
            listen.port()
        );
        return;
    }
    info!(
        port = listen.port(),
        %local_addr,
        "UPnP: game port forwarded automatically"
    );

    // The mapping above already covers the first `LEASE_SECS`; this loop keeps it alive for as
    // long as the server keeps running, rather than letting it quietly expire mid-session.
    let mut interval = tokio::time::interval(Duration::from_secs(u64::from(LEASE_SECS) / 2));
    interval.tick().await; // the first tick fires immediately: already mapped above.
    loop {
        interval.tick().await;
        if let Err(e) = map_once(&gateway, listen.port(), local_addr).await {
            warn!(error = %e, "renewing the UPnP port mapping failed; it may expire soon");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The socket shell. Everything below that decides anything is a pure function further down; this
// part only moves bytes, and is the part CI cannot verify (see the module doc).
// ---------------------------------------------------------------------------------------------

/// Multicast an M-SEARCH for each search target, then fetch and scan whatever device
/// descriptions answer until one advertises a service that can map a port.
async fn discover() -> Result<Gateway, String> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("could not open a UDP socket for SSDP discovery: {e}"))?;
    for target in SEARCH_TARGETS {
        socket
            .send_to(m_search(target).as_bytes(), SSDP_MULTICAST)
            .await
            .map_err(|e| format!("could not send the SSDP discovery datagram: {e}"))?;
    }

    let deadline = tokio::time::Instant::now() + DISCOVERY_TIMEOUT;
    let mut tried: Vec<String> = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let mut buf = [0u8; 1500];
        let wait = RESPONSE_TIMEOUT.min(deadline - tokio::time::Instant::now());
        let Ok(Ok((read, from))) = timeout(wait, socket.recv_from(&mut buf)).await else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&buf[..read]) else {
            continue;
        };
        let Some(location) = parse_location(text) else {
            continue;
        };
        if tried.iter().any(|seen| seen == location) {
            continue;
        }
        debug!(%from, location, "UPnP: fetching a device description");
        tried.push(location.to_string());

        match fetch_gateway(location).await {
            Ok(gateway) => return Ok(gateway),
            Err(e) => debug!(location, error = %e, "UPnP: that device cannot map ports"),
        }
    }
    Err(format!(
        "no UPnP gateway answered within {}s ({} device description(s) tried)",
        DISCOVERY_TIMEOUT.as_secs(),
        tried.len()
    ))
}

/// GET the device description at `location` and turn it into a [`Gateway`], or say why not.
async fn fetch_gateway(location: &str) -> Result<Gateway, String> {
    let (host, port, path) = split_http_url(location)
        .ok_or_else(|| format!("LOCATION `{location}` is not a plain http:// URL"))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: text/xml\r\n\r\n"
    );
    let (status, body) = exchange(host, port, request.into_bytes()).await?;
    if status != 200 {
        return Err(format!("the device description returned HTTP {status}"));
    }
    let xml = String::from_utf8_lossy(&body);
    let (service_type, control_url) = find_wan_service(&xml)
        .ok_or_else(|| "no WAN connection service in the device description".to_string())?;
    let control_url = resolve_control_url(location, &xml, control_url)
        .ok_or_else(|| format!("cannot resolve controlURL `{control_url}`"))?;
    Ok(Gateway {
        control_url,
        service_type: service_type.to_string(),
    })
}

/// POST one `AddPortMapping` to the gateway's control endpoint.
async fn map_once(gateway: &Gateway, port: u16, local_addr: SocketAddr) -> Result<(), String> {
    let (host, control_port, path) = split_http_url(&gateway.control_url)
        .ok_or_else(|| format!("controlURL `{}` is not usable", gateway.control_url))?;
    let body = add_port_mapping_body(
        &gateway.service_type,
        port,
        local_addr,
        LEASE_SECS,
        MAPPING_DESCRIPTION,
    );
    // Sent as one buffer and one write on purpose: miniupnpc's `minisoap.c` carries the note
    // "my old linksys router only took into account soap request that are sent into only one
    // packet", and there is no reason to find out which firmware still behaves that way.
    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}:{control_port}\r\n\
         Content-Type: text/xml; charset=\"utf-8\"\r\n\
         SOAPAction: \"{}#AddPortMapping\"\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        gateway.service_type,
        body.len()
    );
    // A UPnP fault comes back as HTTP 500 with the fault envelope in the body (miniupnpd's own
    // `SoapError` builds it that way), so the status is not what decides success here.
    let (_status, response) = exchange(host, control_port, request.into_bytes()).await?;
    parse_add_port_response(&String::from_utf8_lossy(&response))
}

/// Connect, write the whole request, read the whole (bounded) response, parse its framing.
///
/// `host` must be an IP literal. SSDP is unauthenticated: anything on the LAN can answer an
/// M-SEARCH with any LOCATION it likes, and refusing a hostname here keeps that from turning a
/// boot into an attacker-chosen DNS lookup and HTTP request.
async fn exchange(host: &str, port: u16, request: Vec<u8>) -> Result<(u16, Vec<u8>), String> {
    let ip: IpAddr = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .map_err(|_| format!("refusing to contact `{host}`: not a raw IP address"))?;

    let read = async {
        let mut stream = TcpStream::connect(SocketAddr::new(ip, port)).await?;
        stream.write_all(&request).await?;
        let mut raw = Vec::new();
        stream
            .take(MAX_HTTP_RESPONSE as u64)
            .read_to_end(&mut raw)
            .await?;
        Ok::<_, std::io::Error>(raw)
    };
    let raw = timeout(HTTP_TIMEOUT, read)
        .await
        .map_err(|_| format!("{ip}:{port} did not answer within {}s", HTTP_TIMEOUT.as_secs()))?
        .map_err(|e| format!("{ip}:{port}: {e}"))?;

    parse_http_response(&raw).ok_or_else(|| format!("{ip}:{port} sent an unparsable HTTP response"))
}

/// This machine's own address on whichever interface would actually reach the default route:
/// the address a router's port mapping needs to forward *to*. The well-known "connect a UDP
/// socket, read back its local address" trick: `connect` on a UDP socket only resolves routing
/// and binds a local address for it, it does not send a packet, so this works with no network
/// traffic and no dependency on 8.8.8.8 (or anything else) actually being reachable.
fn local_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The pure part: every decision this module makes, as functions over `&str`. All unit-tested.
// ---------------------------------------------------------------------------------------------

/// The SSDP M-SEARCH datagram for one search target.
///
/// `MX: 3` asks answering devices to spread their replies over up to three seconds, which is what
/// keeps a busy LAN from answering all at once; it is also why [`DISCOVERY_TIMEOUT`] is not one
/// second. Every line ends CRLF and the datagram ends with a blank line, exactly as the UPnP
/// Device Architecture requires and as both `igd-next` and miniupnpc send it.
fn m_search(target: &str) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\n\
         HOST: 239.255.255.250:1900\r\n\
         MAN: \"ssdp:discover\"\r\n\
         MX: 3\r\n\
         ST: {target}\r\n\r\n"
    )
}

/// The `LOCATION` header's value out of an SSDP search response.
///
/// Case-insensitive on the header name, and tolerant of no space after the colon, because real
/// firmware writes it both ways: see the two captured responses in this module's tests.
fn parse_location(response: &str) -> Option<&str> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("location")
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
    })
}

/// Host, port and path of a plain `http://host[:port][/path]` URL. Deliberately not a general URL
/// parser: IGD control traffic is HTTP over the LAN against IP literals, so there is no scheme to
/// choose, no userinfo, no international host to punycode, and nothing to percent-decode.
fn split_http_url(url: &str) -> Option<(&str, u16, &str)> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("HTTP://"))?;
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return None;
    }
    // A colon inside a bare IPv6 literal is not a port separator; a bracketed literal's port
    // always comes after the `]`, which is what the `contains(']')` test rules out.
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty()
                && !port.is_empty()
                && !port.contains(']')
                && port.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (host, port.parse().ok()?)
        }
        _ => (authority, 80),
    };
    Some((host, port, path))
}

/// The first `serviceType`/`controlURL` pair in a device description that can map a port.
///
/// A device description nests `device` inside `deviceList` inside `device` (the port-mapping
/// service lives three levels down, under `WANDevice` then `WANConnectionDevice`), and every
/// level carries services that cannot map ports. Rather than model that tree, this scans every
/// `service` element in document order and takes the first whose `serviceType` is one this
/// module knows how to call, which reaches the same element for every real description without
/// needing to be right about the nesting.
fn find_wan_service(xml: &str) -> Option<(&str, &str)> {
    let mut at = 0;
    while let Some((service, past)) = element(xml, "service", at) {
        at = past;
        let Some((service_type, _)) = element(service, "serviceType", 0) else {
            continue;
        };
        if !WAN_SERVICES.contains(&service_type) {
            continue;
        }
        if let Some((control_url, _)) = element(service, "controlURL", 0)
            && !control_url.is_empty()
        {
            return Some((service_type, control_url));
        }
    }
    None
}

/// Turn a description's `controlURL` into an absolute one.
///
/// An absolute `controlURL` is used as-is. A relative one resolves against the description's own
/// `URLBase` when it has one and against the URL the description was fetched from otherwise,
/// which is what the UPnP Device Architecture says and what miniupnpc does. (`igd-next` ignored
/// `URLBase` outright and always used the SSDP responder's address, which is wrong for any router
/// that serves its description and its control endpoint on different ports.)
fn resolve_control_url(location: &str, xml: &str, control_url: &str) -> Option<String> {
    if control_url.starts_with("http://") || control_url.starts_with("HTTP://") {
        return Some(control_url.to_string());
    }
    let base = element(xml, "URLBase", 0)
        .map(|(base, _)| base)
        .filter(|base| !base.is_empty())
        .unwrap_or(location);
    let (host, port, _) = split_http_url(base)?;
    let separator = if control_url.starts_with('/') { "" } else { "/" };
    Some(format!("http://{host}:{port}{separator}{control_url}"))
}

/// The `AddPortMapping` SOAP envelope.
///
/// The eight arguments are in the order the IGD:1 service template defines them, which is also
/// the order miniupnpc sends (`upnpcommands.c`'s `AddPortMappingArgs`). Nothing here needs XML
/// escaping: every value is a number, an IP literal, or [`MAPPING_DESCRIPTION`], a constant.
fn add_port_mapping_body(
    service_type: &str,
    external_port: u16,
    local_addr: SocketAddr,
    lease_secs: u32,
    description: &str,
) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body>\
         <u:AddPortMapping xmlns:u=\"{service_type}\">\
         <NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{external_port}</NewExternalPort>\
         <NewProtocol>TCP</NewProtocol>\
         <NewInternalPort>{}</NewInternalPort>\
         <NewInternalClient>{}</NewInternalClient>\
         <NewEnabled>1</NewEnabled>\
         <NewPortMappingDescription>{description}</NewPortMappingDescription>\
         <NewLeaseDuration>{lease_secs}</NewLeaseDuration>\
         </u:AddPortMapping>\
         </s:Body>\
         </s:Envelope>",
        local_addr.port(),
        local_addr.ip(),
    )
}

/// Did the router accept the mapping?
///
/// Success is an `AddPortMappingResponse` element, which real firmware sends self-closing and
/// empty. A refusal is a SOAP fault carrying a UPnP `errorCode`/`errorDescription`, which is
/// worth reporting verbatim: 718 (the port is already mapped to another host) and 725 (this
/// router only supports permanent leases) are the two an operator can actually act on.
fn parse_add_port_response(body: &str) -> Result<(), String> {
    if find_tag(body, 0, "AddPortMappingResponse", false).is_some() {
        return Ok(());
    }
    match (
        element(body, "errorCode", 0),
        element(body, "errorDescription", 0),
    ) {
        (Some((code, _)), Some((description, _))) => {
            Err(format!("UPnP error {code} ({description})"))
        }
        (Some((code, _)), None) => Err(format!("UPnP error {code}")),
        _ => Err("the router's reply was neither a success nor a UPnP fault".to_string()),
    }
}

/// Status code and body of an HTTP/1.1 response.
///
/// Handles the three framings an IGD actually uses: `Content-Length` (what miniupnpd, and so most
/// consumer firmware, always sends), `Transfer-Encoding: chunked` (miniupnpc records the BiPAC
/// 7404VNOX as always chunking HTTP/1.1 replies), and close-delimited with neither.
fn parse_http_response(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = &raw[split + 4..];

    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()?;

    let header = |name: &str| {
        head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
        })
    };

    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        return Some((status, dechunk(body)?));
    }
    match header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        // A truncated body is still worth scanning: the cap in `exchange` can cut one short, and
        // the tags this module looks for are near the front.
        Some(len) => Some((status, body[..len.min(body.len())].to_vec())),
        None => Some((status, body.to_vec())),
    }
}

/// Reassemble a `Transfer-Encoding: chunked` body.
fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = body.windows(2).position(|w| w == b"\r\n")?;
        let line = std::str::from_utf8(&body[..end]).ok()?;
        // A chunk size may carry `;ext=value` extensions after a semicolon.
        let size = usize::from_str_radix(line.split(';').next()?.trim(), 16).ok()?;
        body = body.get(end + 2..)?;
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?; // past the chunk's own trailing CRLF
    }
}

/// The text between the first `<tag>` at or after `from` and its matching `</tag>`, plus the
/// offset just past that closing tag so a caller can keep scanning.
fn element<'a>(xml: &'a str, tag: &str, from: usize) -> Option<(&'a str, usize)> {
    let (_, text_start) = find_tag(xml, from, tag, false)?;
    let (text_end, past) = find_tag(xml, text_start, tag, true)?;
    Some((xml[text_start..text_end].trim(), past))
}

/// Byte offsets of the next `<tag ...>` (or `</tag>` when `closing`) at or after `from`, as
/// `(where it starts, one past its `>`)`.
///
/// Deliberately a scanner, not an XML parser: a device description or SOAP envelope only has to
/// be searched for a handful of known element names, so this ignores namespace prefixes (`<u:x>`
/// and `<x>` both match `x`), tolerates attributes and self-closing tags, and never validates
/// anything. The one shape it would misread is a `>` inside an attribute value or an XML comment,
/// neither of which appears in either document.
fn find_tag(xml: &str, from: usize, tag: &str, closing: bool) -> Option<(usize, usize)> {
    let mut at = from;
    while let Some(offset) = xml.get(at..)?.find('<') {
        let start = at + offset;
        let end = start + xml.get(start..)?.find('>')?;
        at = end + 1;

        let inner = xml.get(start + 1..end)?.trim();
        let (is_closing, inner) = match inner.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, inner),
        };
        if is_closing != closing {
            continue;
        }
        let name = inner
            .split([' ', '\t', '\r', '\n', '/'])
            .next()
            .unwrap_or_default();
        if name.rsplit(':').next().unwrap_or(name) == tag {
            return Some((start, at));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real SSDP search response from an Orange/Wanadoo Livebox, quoted verbatim from
    /// miniupnpd's own `minissdp.c` (the comment block above `SendSSDPResponse`, kept there as a
    /// record of what real firmware sends).
    const SSDP_LIVEBOX: &str = "HTTP/1.1 200 OK\r\n\
        CACHE-CONTROL: max-age=1800\r\n\
        DATE: Thu, 01 Jan 1970 04:03:23 GMT\r\n\
        EXT:\r\n\
        LOCATION: http://192.168.0.1:49152/gatedesc.xml\r\n\
        SERVER: Linux/2.4.17, UPnP/1.0, Intel SDK for UPnP devices /1.2\r\n\
        ST: upnp:rootdevice\r\n\
        USN: uuid:75802409-bccb-40e7-8e6c-fa095ecce13e::upnp:rootdevice\r\n\r\n";

    /// The same, from a Linksys 802.11b router, from the same source. Note the header names with
    /// no space after the colon, and the mixed case: this is why [`parse_location`] cannot just
    /// match `"LOCATION: "`.
    const SSDP_LINKSYS: &str = "HTTP/1.1 200 OK\r\n\
        Cache-Control:max-age=120\r\n\
        Location:http://192.168.5.1:5678/rootDesc.xml\r\n\
        Server:NT/5.0 UPnP/1.0\r\n\
        ST:upnp:rootdevice\r\n\
        USN:uuid:upnp-InternetGatewayDevice-1_0-0090a2777777::upnp:rootdevice\r\n\
        EXT:\r\n\r\n";

    /// A real device description from a LINKSYS WAG200G, from miniupnpc's own parser test corpus
    /// (`miniupnpc/testdesc/linksys_WAG200G_desc.xml`, a capture from the real router). The
    /// per-device metadata elements (`manufacturerURL`, `modelDescription`, `serialNumber`, `UPC`
    /// and friends) are elided; every element this module reads, and the whole nesting it has to
    /// see through, is as captured.
    ///
    /// What makes it a real test: the port-mapping service is the *seventh* service element, three
    /// device levels down, behind `Layer3Forwarding:1`, `WANCommonInterfaceConfig:1` and
    /// `WANEthernetLinkConfig:1`, none of which can map a port. It also carries a `URLBase` on a
    /// different port from the description itself.
    const DESC_LINKSYS: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
<specVersion><major>1</major><minor>0</minor></specVersion>
<URLBase>http://192.168.1.1:49152</URLBase>
<device>
<deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
<friendlyName>LINKSYS WAG200G Gateway</friendlyName>
<serviceList>
<service>
<serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType>
<serviceId>urn:upnp-org:serviceId:L3Forwarding1</serviceId>
<controlURL>/upnp/control/L3Forwarding1</controlURL>
<eventSubURL>/upnp/event/L3Forwarding1</eventSubURL>
<SCPDURL>/l3frwd.xml</SCPDURL>
</service>
</serviceList>
<deviceList>
<device>
<deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>
<friendlyName>WANDevice</friendlyName>
<serviceList>
<service>
<serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType>
<serviceId>urn:upnp-org:serviceId:WANCommonIFC1</serviceId>
<controlURL>/upnp/control/WANCommonIFC1</controlURL>
<eventSubURL>/upnp/event/WANCommonIFC1</eventSubURL>
<SCPDURL>/cmnicfg.xml</SCPDURL>
</service>
</serviceList>
<deviceList>
<device>
<deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>
<friendlyName>WANConnectionDevice</friendlyName>
<serviceList>
<service>
<serviceType>urn:schemas-upnp-org:service:WANEthernetLinkConfig:1</serviceType>
<serviceId>urn:upnp-org:serviceId:WANEthLinkC1</serviceId>
<controlURL>/upnp/control/WANEthLinkC1</controlURL>
<eventSubURL>/upnp/event/WANEthLinkC1</eventSubURL>
<SCPDURL>/wanelcfg.xml</SCPDURL>
</service>
<service>
<serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType>
<serviceId>urn:upnp-org:serviceId:WANPPPConn1</serviceId>
<controlURL>/upnp/control/WANPPPConn1</controlURL>
<eventSubURL>/upnp/event/WANPPPConn1</eventSubURL>
<SCPDURL>/pppcfg.xml</SCPDURL>
</service>
</serviceList>
</device>
</deviceList>
</device>
<device>
<deviceType>urn:schemas-upnp-org:device:LANDevice:1</deviceType>
<friendlyName>LANDevice</friendlyName>
<serviceList>
<service>
<serviceType>urn:schemas-upnp-org:service:LANHostConfigManagement:1</serviceType>
<serviceId>urn:upnp-org:serviceId:LANHostCfg1</serviceId>
<controlURL>/upnp/control/LANHostCfg1</controlURL>
<eventSubURL>/upnp/event/LANHostCfg1</eventSubURL>
<SCPDURL>/lanhostc.xml</SCPDURL>
</service>
</serviceList>
</device>
</deviceList>
<presentationURL>http://192.168.1.1/index.htm</presentationURL>
</device>
</root>"#;

    /// A real device description from a Sagemcom Orange Livebox, from the same corpus
    /// (`miniupnpc/testdesc/new_LiveBox_desc.xml`), elided the same way.
    ///
    /// What makes it a real test: it is an `InternetGatewayDevice:2` advertising
    /// `WANPPPConnection:2`, and it carries namespace-prefixed vendor elements
    /// (`<pnpx:X_hardwareId>`, `<df:X_deviceCategory>`) that a naive scanner walks straight into.
    /// `igd-next` would find nothing here: its search target and its service list are both `:1`
    /// only. It has no `URLBase`, so its control URL has to resolve against the LOCATION.
    const DESC_LIVEBOX: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <pnpx:X_hardwareId xmlns:pnpx="http://schemas.microsoft.com/windows/pnpx/2005/11">VEN_0129&amp;DEV_0000&amp;SUBSYS_03&amp;REV_250417</pnpx:X_hardwareId>
    <df:X_deviceCategory xmlns:df="http://schemas.microsoft.com/windows/2008/09/devicefoundation">Network.Gateway</df:X_deviceCategory>
    <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:2</deviceType>
    <friendlyName>Orange Livebox</friendlyName>
    <presentationURL>http://192.168.1.1</presentationURL>
    <iconList>
      <icon>
        <mimetype>image/png</mimetype>
        <url>/87895a19/ligd.png</url>
      </icon>
    </iconList>
    <deviceList>
      <device>
        <deviceType>urn:schemas-upnp-org:device:WANDevice:2</deviceType>
        <friendlyName>WANDevice</friendlyName>
        <serviceList>
          <service>
            <serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType>
            <serviceId>urn:upnp-org:serviceId:WANCommonIFC1</serviceId>
            <controlURL>/87895a19/upnp/control/WANCommonIFC1</controlURL>
            <eventSubURL>/87895a19/upnp/control/WANCommonIFC1</eventSubURL>
            <SCPDURL>/87895a19/gateicfgSCPD.xml</SCPDURL>
          </service>
        </serviceList>
        <deviceList>
          <device>
            <deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:2</deviceType>
            <friendlyName>WANConnectionDevice</friendlyName>
            <serviceList>
              <service>
                <serviceType>urn:schemas-upnp-org:service:WANPPPConnection:2</serviceType>
                <serviceId>urn:upnp-org:serviceId:WANIPConn1</serviceId>
                <controlURL>/87895a19/upnp/control/WANIPConn1</controlURL>
                <eventSubURL>/87895a19/upnp/control/WANIPConn1</eventSubURL>
                <SCPDURL>/87895a19/wanipcSCPD.xml</SCPDURL>
              </service>
            </serviceList>
          </device>
        </deviceList>
      </device>
    </deviceList>
  </device>
</root>"#;

    /// The body a router sends when `AddPortMapping` worked, in the exact shape miniupnpd builds
    /// it (`upnpsoap.c`'s `AddPortMapping`: `"<u:%sResponse xmlns:u=\"%s\"/>"`, self-closing and
    /// empty), inside the envelope its `BuildSendAndCloseSoapResp` wraps every response in.
    const SOAP_OK: &str = "<?xml version=\"1.0\"?>\r\n\
        <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
        s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
        <s:Body><u:AddPortMappingResponse \
        xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\"/></s:Body></s:Envelope>";

    /// A refusal, in the exact shape miniupnpd's `SoapError` builds (`upnpsoap.c`). 718
    /// `ConflictInMappingEntry` is the IGD:1 service template's own error for "that external port
    /// is already mapped to a different host", which is the refusal an operator most often hits.
    const SOAP_FAULT: &str = "<s:Envelope \
        xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
        s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
        <s:Body><s:Fault><faultcode>s:Client</faultcode>\
        <faultstring>UPnPError</faultstring><detail>\
        <UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\">\
        <errorCode>718</errorCode>\
        <errorDescription>ConflictInMappingEntry</errorDescription>\
        </UPnPError></detail></s:Fault></s:Body></s:Envelope>";

    #[test]
    fn a_loopback_listen_address_is_skipped_without_touching_the_network() {
        // Not asserting on `attempt` directly (it is `async` and reaches the real network for
        // anything past this check): just confirming the guard this function opens with reads
        // `is_loopback` the way it is meant to, on the exact addresses `--listen 127.0.0.1:...`
        // and the default `0.0.0.0:...` actually produce.
        assert!(
            "127.0.0.1:7777"
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
                .is_loopback()
        );
        assert!(
            !"0.0.0.0:7777"
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
                .is_loopback()
        );
    }

    #[test]
    fn local_ipv4_finds_some_address_or_cleanly_says_it_cannot() {
        // The environment this runs in may or may not have a real default route (a sandboxed CI
        // runner sometimes does not): both outcomes are legitimate, so this only asserts the
        // function does not panic and, when it does find something, that it looks like a real
        // IPv4 address rather than a placeholder.
        if let Some(ip) = local_ipv4() {
            assert!(!ip.is_unspecified(), "0.0.0.0 is not a real local address");
        }
    }

    #[test]
    fn the_m_search_datagram_is_shaped_the_way_the_device_architecture_requires() {
        let datagram = m_search(SEARCH_TARGETS[0]);
        assert!(datagram.starts_with("M-SEARCH * HTTP/1.1\r\n"));
        assert!(datagram.contains("\r\nHOST: 239.255.255.250:1900\r\n"));
        assert!(datagram.contains("\r\nMAN: \"ssdp:discover\"\r\n"));
        assert!(datagram.contains("\r\nMX: 3\r\n"));
        assert!(datagram.contains(
            "\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n"
        ));
        // The blank line that ends the datagram: without it, a device ignores the search.
        assert!(datagram.ends_with("\r\n\r\n"));
        // Every line break is CRLF, never a bare LF.
        assert_eq!(datagram.matches('\n').count(), datagram.matches("\r\n").count());
    }

    #[test]
    fn the_location_header_is_found_in_both_shapes_real_firmware_writes_it() {
        assert_eq!(
            parse_location(SSDP_LIVEBOX),
            Some("http://192.168.0.1:49152/gatedesc.xml"),
            "`LOCATION: ` with a space, uppercase"
        );
        assert_eq!(
            parse_location(SSDP_LINKSYS),
            Some("http://192.168.5.1:5678/rootDesc.xml"),
            "`Location:` with no space, mixed case"
        );
        assert_eq!(
            parse_location("HTTP/1.1 200 OK\r\nST:upnp:rootdevice\r\n\r\n"),
            None,
            "a response with no LOCATION at all is not a gateway"
        );
        assert_eq!(
            parse_location("HTTP/1.1 200 OK\r\nLOCATION:\r\n\r\n"),
            None,
            "an empty LOCATION is not a URL"
        );
    }

    #[test]
    fn a_lan_http_url_splits_into_host_port_and_path() {
        assert_eq!(
            split_http_url("http://192.168.0.1:49152/gatedesc.xml"),
            Some(("192.168.0.1", 49152, "/gatedesc.xml"))
        );
        assert_eq!(
            split_http_url("http://192.168.1.1/rootDesc.xml"),
            Some(("192.168.1.1", 80, "/rootDesc.xml")),
            "no port means 80"
        );
        assert_eq!(
            split_http_url("http://192.168.1.1:49152"),
            Some(("192.168.1.1", 49152, "/")),
            "no path means /"
        );
        assert_eq!(
            split_http_url("http://[fe80::1]:5000/desc.xml"),
            Some(("[fe80::1]", 5000, "/desc.xml")),
            "a bracketed IPv6 literal keeps its brackets and gives up its port"
        );
        assert_eq!(
            split_http_url("http://[fe80::1]/desc.xml"),
            Some(("[fe80::1]", 80, "/desc.xml")),
            "the colons inside an IPv6 literal are not a port separator"
        );
        assert_eq!(split_http_url("https://192.168.1.1/desc.xml"), None);
        assert_eq!(split_http_url("192.168.1.1/desc.xml"), None);
    }

    #[test]
    fn the_port_mapping_service_is_found_in_a_real_linksys_description() {
        assert_eq!(
            find_wan_service(DESC_LINKSYS),
            Some((
                "urn:schemas-upnp-org:service:WANPPPConnection:1",
                "/upnp/control/WANPPPConn1"
            )),
            "the seventh service element, three device levels down, past four that cannot map \
             a port"
        );
    }

    #[test]
    fn the_port_mapping_service_is_found_in_a_real_igd2_livebox_description() {
        assert_eq!(
            find_wan_service(DESC_LIVEBOX),
            Some((
                "urn:schemas-upnp-org:service:WANPPPConnection:2",
                "/87895a19/upnp/control/WANIPConn1"
            )),
            "an IGD:2 device, and namespace-prefixed vendor elements before it"
        );
    }

    #[test]
    fn a_description_with_nothing_that_can_map_a_port_finds_nothing() {
        let printer = "<root><device><serviceList><service>\
            <serviceType>urn:schemas-upnp-org:service:PrintBasic:1</serviceType>\
            <controlURL>/print</controlURL></service></serviceList></device></root>";
        assert_eq!(find_wan_service(printer), None);
    }

    #[test]
    fn the_control_url_resolves_against_url_base_when_the_description_has_one() {
        // The Linksys serves its description from port 80 but declares URLBase on 49152: taking
        // the LOCATION's port instead (which is what `igd-next` did) would POST to the wrong one.
        assert_eq!(
            resolve_control_url(
                "http://192.168.1.1/rootDesc.xml",
                DESC_LINKSYS,
                "/upnp/control/WANPPPConn1"
            )
            .as_deref(),
            Some("http://192.168.1.1:49152/upnp/control/WANPPPConn1")
        );
    }

    #[test]
    fn the_control_url_resolves_against_the_location_when_there_is_no_url_base() {
        assert_eq!(
            resolve_control_url(
                "http://192.168.1.1:49152/87895a19/gatedesc.xml",
                DESC_LIVEBOX,
                "/87895a19/upnp/control/WANIPConn1"
            )
            .as_deref(),
            Some("http://192.168.1.1:49152/87895a19/upnp/control/WANIPConn1")
        );
    }

    #[test]
    fn an_absolute_control_url_is_left_alone_and_a_relative_one_gets_its_slash() {
        assert_eq!(
            resolve_control_url(
                "http://192.168.1.1/desc.xml",
                "<root/>",
                "http://192.168.1.1:5000/ctl"
            )
            .as_deref(),
            Some("http://192.168.1.1:5000/ctl")
        );
        assert_eq!(
            resolve_control_url("http://192.168.1.1/desc.xml", "<root/>", "ctl").as_deref(),
            Some("http://192.168.1.1:80/ctl")
        );
    }

    #[test]
    fn the_add_port_mapping_envelope_carries_the_eight_arguments_in_the_spec_s_order() {
        let body = add_port_mapping_body(
            "urn:schemas-upnp-org:service:WANPPPConnection:1",
            7777,
            "192.168.1.50:7777".parse().unwrap(),
            7_200,
            "terrustia",
        );
        // The service type the router itself advertised, echoed back at it.
        assert!(
            body.contains(
                "<u:AddPortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANPPPConnection:1\">"
            ),
            "{body}"
        );
        // Argument order is the IGD:1 service template's own, matching miniupnpc's
        // `AddPortMappingArgs`. Checked as one substring so a reordering fails the test.
        assert!(
            body.contains(
                "<NewRemoteHost></NewRemoteHost>\
                 <NewExternalPort>7777</NewExternalPort>\
                 <NewProtocol>TCP</NewProtocol>\
                 <NewInternalPort>7777</NewInternalPort>\
                 <NewInternalClient>192.168.1.50</NewInternalClient>\
                 <NewEnabled>1</NewEnabled>\
                 <NewPortMappingDescription>terrustia</NewPortMappingDescription>\
                 <NewLeaseDuration>7200</NewLeaseDuration>"
            ),
            "{body}"
        );
        assert!(body.contains("</u:AddPortMapping></s:Body></s:Envelope>"), "{body}");
    }

    #[test]
    fn the_envelope_maps_the_external_port_to_a_different_internal_one_when_asked() {
        // Not a case the server itself produces (it maps a port to itself), but the one place a
        // silent argument mix-up between external and internal would hide.
        let body = add_port_mapping_body(
            "urn:schemas-upnp-org:service:WANIPConnection:1",
            7777,
            "10.0.0.4:1234".parse().unwrap(),
            0,
            "terrustia",
        );
        assert!(body.contains("<NewExternalPort>7777</NewExternalPort>"), "{body}");
        assert!(body.contains("<NewInternalPort>1234</NewInternalPort>"), "{body}");
        assert!(body.contains("<NewLeaseDuration>0</NewLeaseDuration>"), "{body}");
    }

    #[test]
    fn a_real_success_response_reads_as_success() {
        assert_eq!(parse_add_port_response(SOAP_OK), Ok(()));
    }

    #[test]
    fn a_real_soap_fault_reads_as_its_upnp_error_code_and_description() {
        assert_eq!(
            parse_add_port_response(SOAP_FAULT),
            Err("UPnP error 718 (ConflictInMappingEntry)".to_string())
        );
    }

    #[test]
    fn a_reply_that_is_neither_is_reported_as_neither() {
        let err = parse_add_port_response("<html><body>404 Not Found</body></html>").unwrap_err();
        assert!(err.contains("neither a success nor a UPnP fault"), "{err}");
    }

    #[test]
    fn a_content_length_framed_fault_response_is_read_off_the_wire_whole() {
        // miniupnpd answers a UPnP fault with HTTP 500 and always sets Content-Length
        // (`upnphttp.c`'s `httpresphead`), so the status must not be what decides success.
        let raw = format!(
            "HTTP/1.1 500 Internal Server Error\r\n\
             Content-Type: text/xml; charset=\"utf-8\"\r\n\
             Connection: close\r\n\
             Content-Length: {}\r\n\
             Server: Linux/3.10 UPnP/1.1 MiniUPnPd/2.3.7\r\n\
             Ext:\r\n\r\n{SOAP_FAULT}",
            SOAP_FAULT.len()
        );
        let (status, body) = parse_http_response(raw.as_bytes()).unwrap();
        assert_eq!(status, 500);
        assert_eq!(String::from_utf8(body).unwrap(), SOAP_FAULT);
    }

    #[test]
    fn a_chunked_response_is_reassembled() {
        // miniupnpc records the BiPAC 7404VNOX as always chunking its HTTP/1.1 replies
        // (`minisoap.c`'s own comment), so this framing is not hypothetical.
        let (first, second) = SOAP_OK.split_at(40);
        let raw = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/xml\r\n\
             Transfer-Encoding: chunked\r\n\r\n\
             {:x}\r\n{first}\r\n{:x}\r\n{second}\r\n0\r\n\r\n",
            first.len(),
            second.len()
        );
        let (status, body) = parse_http_response(raw.as_bytes()).unwrap();
        assert_eq!(status, 200);
        assert_eq!(String::from_utf8(body).unwrap(), SOAP_OK);
        // And the reassembled body still reads as the success it is.
        assert_eq!(parse_add_port_response(SOAP_OK), Ok(()));
    }

    #[test]
    fn a_chunk_size_with_an_extension_is_still_a_chunk_size() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;name=v\r\nhello\r\n0\r\n\r\n";
        let (_, body) = parse_http_response(raw).unwrap();
        assert_eq!(body, b"hello");
    }

    #[test]
    fn a_close_delimited_response_with_no_length_header_is_taken_whole() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\n\r\n<root/>";
        assert_eq!(parse_http_response(raw), Some((200, b"<root/>".to_vec())));
    }

    #[test]
    fn a_response_with_no_header_terminator_is_refused_rather_than_guessed_at() {
        assert_eq!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n"), None);
        assert_eq!(parse_http_response(b"garbage\r\n\r\nbody"), None);
    }

    #[test]
    fn the_tag_scanner_ignores_namespace_prefixes_and_attributes() {
        // The returned offset is one past `</b>`, so a caller resumes at `</a>`.
        assert_eq!(element("<a><b>x</b></a>", "b", 0), Some(("x", 11)));
        assert_eq!(element("<s:Body><u:Ok>y</u:Ok></s:Body>", "Ok", 0).map(|e| e.0), Some("y"));
        assert_eq!(
            element("<controlURL xml:lang=\"en\">/ctl</controlURL>", "controlURL", 0).map(|e| e.0),
            Some("/ctl")
        );
        assert_eq!(element("<a>x</a>", "b", 0), None);
        // A self-closing element has no text, so `element` finds nothing, but `find_tag` sees it.
        assert_eq!(element("<u:Resp xmlns:u=\"x\"/>", "Resp", 0), None);
        assert!(find_tag("<u:Resp xmlns:u=\"x\"/>", 0, "Resp", false).is_some());
    }
}
