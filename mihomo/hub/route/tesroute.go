package route

import (
	"bufio"
	"bytes"
	"compress/flate"
	"compress/gzip"
	"compress/zlib"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/netip"
	"net/url"
	"os"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/andybalholm/brotli"
	"github.com/klauspost/compress/zstd"
	"github.com/metacubex/chi"
	"github.com/metacubex/chi/render"
	"github.com/metacubex/http"
	"github.com/metacubex/mihomo/adapter"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/tunnel"
	tls "github.com/metacubex/tls"
	utls "github.com/metacubex/utls"
)

const (
	// Sorted non-GREASE cipher-suite SHA-256 prefix matching Bun 1.4.x BoringSSL:
	// JA4 = t13d1713h1_5b57614c22b0_...
	opencodeJA4CipherHash = "5b57614c22b0"
	defaultTraceURL       = "https://www.cloudflare.com/cdn-cgi/trace"
)

type relayTransportKey struct {
	proxyName       string
	adapterPtr      uintptr
	fingerprintMode string
	skipVerify      bool
}

var (
	relayTransportCache sync.Map // map[relayTransportKey]http.RoundTripper
	directOutbound      = mustBuildDirectProxy()
	ipv4FallbackRegex   = regexp.MustCompile(`\b(?:\d{1,3}\.){3}\d{1,3}\b`)
)

func mustBuildDirectProxy() C.Proxy {
	p, err := adapter.ParseProxy(map[string]any{
		"name": "TESIE_INTERNAL_DIRECT",
		"type": "direct",
	})
	if err != nil {
		panic(fmt.Sprintf("failed to construct internal direct proxy: %v", err))
	}
	return p
}

// newOpenCodeClientHelloSpec returns the exact TLS ClientHelloSpec captured from
// Bun 1.4.3's BoringSSL runtime (used by official OpenCode 1.x & 2.x CLI builds):
//   - JA4 = t13d1713h1_5b57614c22b0_...
//   - 17 cipher suites, no GREASE, no padding
//   - ALPN: ["http/1.1"] ONLY (Bun's fetch speaks HTTP/1.1 only)
//   - 13 extensions in exact BoringSSL order including status_request (0x0005),
//     signed_certificate_timestamp (0x0012), and X25519MLKEM768 first keyshare.
func newOpenCodeClientHelloSpec() utls.ClientHelloSpec {
	return utls.ClientHelloSpec{
		TLSVersMin: utls.VersionTLS12,
		TLSVersMax: utls.VersionTLS13,
		CipherSuites: []uint16{
			utls.TLS_AES_128_GCM_SHA256,
			utls.TLS_AES_256_GCM_SHA384,
			utls.TLS_CHACHA20_POLY1305_SHA256,
			utls.TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
			utls.TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
			utls.TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
			utls.TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
			utls.TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
			utls.TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
			utls.TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA,
			utls.TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA,
			utls.TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA,
			utls.TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA,
			utls.TLS_RSA_WITH_AES_128_GCM_SHA256,
			utls.TLS_RSA_WITH_AES_256_GCM_SHA384,
			utls.TLS_RSA_WITH_AES_128_CBC_SHA,
			utls.TLS_RSA_WITH_AES_256_CBC_SHA,
		},
		CompressionMethods: []uint8{0},
		Extensions: []utls.TLSExtension{
			&utls.SNIExtension{},
			&utls.ExtendedMasterSecretExtension{},
			&utls.RenegotiationInfoExtension{Renegotiation: utls.RenegotiateOnceAsClient},
			&utls.SupportedCurvesExtension{Curves: []utls.CurveID{
				utls.X25519MLKEM768,
				utls.X25519,
				utls.CurveP256,
				utls.CurveP384,
			}},
			&utls.SupportedPointsExtension{SupportedPoints: []uint8{0}},
			&utls.SessionTicketExtension{},
			&utls.ALPNExtension{AlpnProtocols: []string{"http/1.1"}},
			&utls.StatusRequestExtension{},
			&utls.SignatureAlgorithmsExtension{SupportedSignatureAlgorithms: []utls.SignatureScheme{
				utls.ECDSAWithP256AndSHA256,
				utls.PSSWithSHA256,
				utls.PKCS1WithSHA256,
				utls.ECDSAWithP384AndSHA384,
				utls.PSSWithSHA384,
				utls.PKCS1WithSHA384,
				utls.PSSWithSHA512,
				utls.PKCS1WithSHA512,
				utls.PKCS1WithSHA1,
			}},
			&utls.SCTExtension{},
			&utls.KeyShareExtension{KeyShares: []utls.KeyShare{
				{Group: utls.X25519MLKEM768},
				{Group: utls.X25519},
			}},
			&utls.PSKKeyExchangeModesExtension{Modes: []uint8{utls.PskModeDHE}},
			&utls.SupportedVersionsExtension{Versions: []uint16{
				utls.VersionTLS13,
				utls.VersionTLS12,
			}},
		},
	}
}

func init() {
	Register(func(r chi.Router) {
		r.Mount("/tesie", tesrouteRouter())
		r.Mount("/tesroute", tesrouteRouter())
	})
}

func dialOpenCodeUTLS(ctx context.Context, rawConn net.Conn, serverName string, skipVerify bool) (net.Conn, error) {
	cfg := &utls.Config{
		ServerName:         serverName,
		InsecureSkipVerify: skipVerify,
		MinVersion:         utls.VersionTLS12,
	}
	uconn := utls.UClient(rawConn, cfg, utls.HelloCustom)
	spec := newOpenCodeClientHelloSpec()
	if err := uconn.ApplyPreset(&spec); err != nil {
		_ = rawConn.Close()
		return nil, fmt.Errorf("apply OpenCode uTLS preset: %w", err)
	}
	if err := uconn.HandshakeContext(ctx); err != nil {
		_ = rawConn.Close()
		return nil, err
	}
	return uconn, nil
}

func resolveProxyAdapter(name string) (C.Proxy, bool) {
	trimmed := strings.TrimSpace(name)
	if trimmed == "" || strings.EqualFold(trimmed, "DIRECT") {
		if p, ok := tunnel.Proxies()["DIRECT"]; ok {
			return p, true
		}
		return directOutbound, true
	}
	p, ok := tunnel.Proxies()[trimmed]
	return p, ok
}

func buildMetadata(host string, port string) (*C.Metadata, error) {
	p, err := strconv.ParseUint(port, 10, 16)
	if err != nil {
		return nil, fmt.Errorf("invalid port %q: %w", port, err)
	}
	md := &C.Metadata{
		NetWork: C.TCP,
		DstPort: uint16(p),
	}
	if ip, err := netip.ParseAddr(host); err == nil {
		md.DstIP = ip
	} else {
		md.Host = host
	}
	return md, nil
}

func splitAddrDefaultPort(addr string, defaultPort string) (string, string, error) {
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		return strings.Trim(addr, "[]"), defaultPort, nil
	}
	if port == "" {
		port = defaultPort
	}
	return host, port, nil
}

func dialProxyRawConn(ctx context.Context, proxy C.Proxy, host string, port string) (net.Conn, error) {
	md, err := buildMetadata(host, port)
	if err != nil {
		return nil, err
	}
	dialCtx, cancel := context.WithTimeout(ctx, 14*time.Second)
	defer cancel()
	return proxy.DialContext(dialCtx, md)
}

// ============================================================================
// BunTransport: HTTP/1.1 wire-accurate Bun `fetch` transport over Mihomo proxies
// ============================================================================
//
// Why this exists:
//   1. Go's standard http.Transport canonicalizes header names (`x-opencode-session` ->
//      `X-Opencode-Session`), writes `Host` first, sends `Accept-Encoding: gzip`,
//      and advertises h2 ALPN. Official OpenCode CLI builds run on Bun, whose WebKit
//      HTTP writer emits known headers first (`Authorization`, `Content-Length`,
//      `Content-Type`, `User-Agent`), then lowercased custom headers sorted
//      alphabetically (`x-opencode-*`), followed by `Connection: keep-alive`,
//      `Accept: */*`, `Host: ...`, and `Accept-Encoding: gzip, deflate, br, zstd`.
//   2. Idle pooled connections run a background 1-byte Peek watcher so connections
//      closed by upstream or SOCKS5 proxies while idle are evicted immediately, and
//      any reused connection that drops before sending a response byte is retried
//      once on a fresh TCP/TLS tunnel (eliminating spurious `Post "...": EOF` errors).

var bunCommonHeaders = []string{
	"Accept", "Accept-Charset", "Accept-Encoding", "Accept-Language", "Authorization", "Cache-Control",
	"Connection", "Content-Encoding", "Content-Language", "Content-Length", "Content-Type", "Cookie", "DNT",
	"Expect", "Host", "If-None-Match", "Keep-Alive", "Last-Event-Id", "Origin", "Pragma",
	"Proxy-Authorization", "Range", "Referer", "Sec-Fetch-Mode", "TE", "Upgrade-Insecure-Requests",
	"User-Agent",
}

type bunRelayTransport struct {
	proxy                 C.Proxy
	skipVerify            bool
	responseHeaderTimeout time.Duration
	idleTimeout           time.Duration
	maxIdlePerHost        int

	mu   sync.Mutex
	idle map[string][]*bunConn
}

type bunConn struct {
	net.Conn
	key      string
	br       *bufio.Reader
	bw       *bufio.Writer
	idleAt   time.Time
	timer    *time.Timer
	watched  chan struct{}
	watchErr error
}

func newBunRelayTransport(proxy C.Proxy, skipVerify bool) *bunRelayTransport {
	return &bunRelayTransport{
		proxy:                 proxy,
		skipVerify:            skipVerify,
		responseHeaderTimeout: 45 * time.Second,
		idleTimeout:           20 * time.Second,
		maxIdlePerHost:        8,
		idle:                  make(map[string][]*bunConn),
	}
}

func (t *bunRelayTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	body, err := readBunRequestBody(req)
	if err != nil {
		return nil, err
	}
	u := req.URL
	if u == nil || u.Host == "" || (u.Scheme != "https" && u.Scheme != "http") {
		return nil, fmt.Errorf("bun transport: unsupported URL %q", u)
	}
	head, decompress, err := bunRequestHead(req, body)
	if err != nil {
		return nil, err
	}
	ctx := req.Context()
	port := u.Port()
	if port == "" {
		if u.Scheme == "http" {
			port = "80"
		} else {
			port = "443"
		}
	}
	hostname := u.Hostname()
	key := u.Scheme + "://" + net.JoinHostPort(hostname, port)

	for retried := false; ; retried = true {
		c, wasReused, err := t.getConn(ctx, u.Scheme, key, hostname, port)
		if err != nil {
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			return nil, err
		}
		resp, dropped, err := t.exchange(ctx, c, req, head, body, decompress)
		if err == nil {
			return resp, nil
		}
		// If a pooled keep-alive connection was closed by the SOCKS5/HTTP proxy or
		// upstream right before the first response byte, dial one fresh connection.
		if wasReused && dropped && !retried && ctx.Err() == nil {
			continue
		}
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, err
	}
}

func readBunRequestBody(req *http.Request) ([]byte, error) {
	if req.Body == nil || req.Body == http.NoBody {
		return nil, nil
	}
	defer req.Body.Close()
	return io.ReadAll(req.Body)
}

func (t *bunRelayTransport) getConn(ctx context.Context, scheme, key, hostname, port string) (*bunConn, bool, error) {
	for c := t.takeIdle(key); c != nil; c = t.takeIdle(key) {
		if c.reclaim() {
			return c, true, nil
		}
		_ = c.Close()
	}

	rawConn, err := dialProxyRawConn(ctx, t.proxy, hostname, port)
	if err != nil {
		return nil, false, err
	}
	conn := rawConn
	if scheme == "https" {
		tlsCtx, cancel := context.WithTimeout(ctx, 14*time.Second)
		conn, err = dialOpenCodeUTLS(tlsCtx, rawConn, hostname, t.skipVerify)
		cancel()
		if err != nil {
			return nil, false, err
		}
	}
	c := &bunConn{
		Conn: conn,
		key:  key,
		br:   bufio.NewReaderSize(conn, 32*1024),
		bw:   bufio.NewWriterSize(conn, 32*1024),
	}
	return c, false, nil
}

func (t *bunRelayTransport) exchange(
	ctx context.Context,
	c *bunConn,
	req *http.Request,
	head []byte,
	body []byte,
	decompress bool,
) (*http.Response, bool, error) {
	doneCh := make(chan struct{})
	var closeOnce sync.Once
	forceClose := func() {
		closeOnce.Do(func() {
			_ = c.Close()
		})
	}
	var watcherWg sync.WaitGroup
	watcherWg.Add(1)
	go func() {
		defer watcherWg.Done()
		select {
		case <-ctx.Done():
			forceClose()
		case <-doneCh:
		}
	}()

	stopWatcher := func() bool {
		close(doneCh)
		watcherWg.Wait()
		return ctx.Err() == nil
	}

	fail := func(err error, dropped bool) (*http.Response, bool, error) {
		stopWatcher()
		forceClose()
		var netErr net.Error
		if t.responseHeaderTimeout > 0 && ctx.Err() == nil && errors.As(err, &netErr) && netErr.Timeout() {
			err = fmt.Errorf("timeout awaiting upstream response headers: %w", err)
		}
		return nil, dropped, err
	}

	_, err := c.bw.Write(head)
	if err == nil && len(body) > 0 {
		_, err = c.bw.Write(body)
	}
	if err == nil {
		err = c.bw.Flush()
	}
	if err != nil {
		return fail(err, isBunConnDropped(err))
	}

	if t.responseHeaderTimeout > 0 {
		_ = c.SetReadDeadline(time.Now().Add(t.responseHeaderTimeout))
	}
	if _, err := c.br.Peek(1); err != nil {
		return fail(err, isBunConnDropped(err))
	}

	resp, err := http.ReadResponse(c.br, req)
	for err == nil && resp.StatusCode < 200 && resp.StatusCode != http.StatusSwitchingProtocols {
		resp, err = http.ReadResponse(c.br, req)
	}
	if err != nil {
		return fail(err, false)
	}
	_ = c.SetReadDeadline(time.Time{})

	raw := &bunBody{
		t:          t,
		c:          c,
		ctx:        ctx,
		stop:       stopWatcher,
		forceClose: forceClose,
		src:        resp.Body,
		reusable:   !req.Close && !resp.Close,
	}
	if resp.Body == http.NoBody {
		raw.finish(true)
		return resp, false, nil
	}
	resp.Body = raw
	if encoding := strings.ToLower(strings.TrimSpace(resp.Header.Get("Content-Encoding"))); decompress && isBunEncoding(encoding) {
		resp.Body = &bunDecoder{body: raw, encoding: encoding}
		resp.Header.Del("Content-Encoding")
		resp.Header.Del("Content-Length")
		resp.ContentLength = -1
		resp.Uncompressed = true
	}
	return resp, false, nil
}

func isBunConnDropped(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, io.EOF) ||
		errors.Is(err, io.ErrUnexpectedEOF) ||
		errors.Is(err, syscall.ECONNRESET) ||
		errors.Is(err, syscall.ECONNABORTED) ||
		errors.Is(err, syscall.EPIPE) {
		return true
	}
	msg := strings.ToLower(err.Error())
	return strings.Contains(msg, "forcibly closed") ||
		strings.Contains(msg, "connection reset") ||
		strings.Contains(msg, "broken pipe") ||
		strings.Contains(msg, "eof")
}

func (t *bunRelayTransport) takeIdle(key string) *bunConn {
	t.mu.Lock()
	defer t.mu.Unlock()
	conns := t.idle[key]
	if len(conns) == 0 {
		return nil
	}
	c := conns[len(conns)-1]
	t.setIdleLocked(key, conns[:len(conns)-1])
	return c
}

func (t *bunRelayTransport) putIdle(c *bunConn) {
	t.mu.Lock()
	if len(t.idle[c.key]) >= t.maxIdlePerHost {
		t.mu.Unlock()
		_ = c.Close()
		return
	}
	c.idleAt = time.Now()
	c.watched = make(chan struct{})
	c.timer = nil
	if t.idleTimeout > 0 {
		c.timer = time.AfterFunc(t.idleTimeout, func() {
			if t.removeIdle(c) {
				_ = c.Close()
			}
		})
	}
	t.idle[c.key] = append(t.idle[c.key], c)
	t.mu.Unlock()

	go func() {
		_, c.watchErr = c.br.Peek(1)
		if t.removeIdle(c) {
			_ = c.Close()
		}
		close(c.watched)
	}()
}

func (t *bunRelayTransport) removeIdle(c *bunConn) bool {
	t.mu.Lock()
	defer t.mu.Unlock()
	conns := t.idle[c.key]
	idx := -1
	for i, item := range conns {
		if item == c {
			idx = i
			break
		}
	}
	if idx < 0 {
		return false
	}
	next := append(conns[:idx], conns[idx+1:]...)
	t.setIdleLocked(c.key, next)
	return true
}

func (t *bunRelayTransport) setIdleLocked(key string, conns []*bunConn) {
	if len(conns) == 0 {
		delete(t.idle, key)
		return
	}
	t.idle[key] = conns
}

func (c *bunConn) reclaim() bool {
	if c.timer != nil {
		c.timer.Stop()
	}
	_ = c.SetReadDeadline(time.Unix(1, 0))
	<-c.watched
	return errors.Is(c.watchErr, os.ErrDeadlineExceeded) && c.SetReadDeadline(time.Time{}) == nil
}

type bunBody struct {
	t          *bunRelayTransport
	c          *bunConn
	ctx        context.Context
	stop       func() bool
	forceClose func()
	src        io.Reader
	reusable   bool
	once       sync.Once
}

func (b *bunBody) Read(p []byte) (int, error) {
	n, err := b.src.Read(p)
	if err == io.EOF {
		b.finish(true)
	} else if err != nil {
		if b.ctx.Err() != nil {
			err = b.ctx.Err()
		}
		b.finish(false)
	}
	return n, err
}

func (b *bunBody) Close() error {
	b.finish(false)
	return nil
}

func (b *bunBody) finish(ended bool) {
	b.once.Do(func() {
		stillLive := b.stop()
		if stillLive && ended && b.reusable {
			b.t.putIdle(b.c)
			return
		}
		b.forceClose()
	})
}

func isBunEncoding(encoding string) bool {
	switch encoding {
	case "gzip", "x-gzip", "deflate", "br", "zstd":
		return true
	}
	return false
}

type bunDecoder struct {
	mu       sync.Mutex
	body     *bunBody
	encoding string
	reader   io.Reader
	release  func()
	err      error
}

func (d *bunDecoder) Read(p []byte) (int, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.err != nil {
		return 0, d.err
	}
	if d.reader == nil {
		if d.reader, d.release, d.err = newBunStreamDecoder(d.encoding, d.body); d.err != nil {
			_ = d.body.Close()
			return 0, d.err
		}
	}
	n, err := d.reader.Read(p)
	if err != nil {
		d.err = err
		if d.release != nil {
			d.release()
			d.release = nil
		}
		if err == io.EOF {
			_, _ = io.Copy(io.Discard, io.LimitReader(d.body, 4<<10))
		}
	}
	return n, err
}

func (d *bunDecoder) Close() error {
	err := d.body.Close()
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.release != nil {
		d.release()
		d.release = nil
	}
	if d.err == nil {
		d.err = http.ErrBodyReadAfterClose
	}
	return err
}

func newBunStreamDecoder(encoding string, r io.Reader) (io.Reader, func(), error) {
	switch encoding {
	case "deflate":
		reader := bufio.NewReader(r)
		hdr, err := reader.Peek(2)
		if err != nil {
			return nil, nil, err
		}
		if hdr[0]&0x0f == 8 && (uint16(hdr[0])<<8|uint16(hdr[1]))%31 == 0 {
			dec, err := zlib.NewReader(reader)
			if err != nil {
				return nil, nil, err
			}
			return dec, func() { _ = dec.Close() }, nil
		}
		dec := flate.NewReader(reader)
		return dec, func() { _ = dec.Close() }, nil
	case "br":
		return brotli.NewReader(r), nil, nil
	case "zstd":
		dec, err := zstd.NewReader(r, zstd.WithDecoderConcurrency(1), zstd.WithDecoderMaxWindow(8<<20))
		if err != nil {
			return nil, nil, err
		}
		return dec, dec.Close, nil
	default:
		dec, err := gzip.NewReader(r)
		if err != nil {
			return nil, nil, err
		}
		return dec, func() { _ = dec.Close() }, nil
	}
}

func bunRequestHead(req *http.Request, body []byte) ([]byte, bool, error) {
	values := make(map[string]string, len(req.Header))
	for name, list := range req.Header {
		for _, value := range list {
			if !validBunHeaderField(name, value) {
				return nil, false, fmt.Errorf("bun transport: invalid header field %q", name)
			}
		}
		lower := strings.ToLower(name)
		if len(list) == 0 || lower == "host" || lower == "connection" || lower == "content-length" || lower == "transfer-encoding" {
			continue
		}
		value := strings.Join(list, ", ")
		if previous, ok := values[lower]; ok {
			value = previous + ", " + value
		}
		values[lower] = value
	}
	if values["accept"] == "*/*" {
		delete(values, "accept")
	}
	_, hasUserAgent := values["user-agent"]
	_, hasAccept := values["accept"]
	_, hasEncoding := values["accept-encoding"]
	method := req.Method
	if method == "" {
		method = http.MethodGet
	}
	if !validBunHeaderField(method, "") {
		return nil, false, errors.New("bun transport: invalid request method")
	}
	requestURI := req.URL.RequestURI()
	for i := 0; i < len(requestURI); i++ {
		if requestURI[i] <= ' ' || requestURI[i] == 0x7f {
			return nil, false, errors.New("bun transport: invalid request URI")
		}
	}
	host := req.Host
	if host == "" {
		host = req.URL.Host
	}
	if host == "" || !validBunHeaderField("Host", host) || strings.ContainsAny(host, " \t/\\#?@") {
		return nil, false, errors.New("bun transport: invalid request host")
	}

	var head strings.Builder
	head.WriteString(method + " " + requestURI + " HTTP/1.1\r\n")
	wroteLength := false
	for _, name := range bunCommonHeaders {
		lower := strings.ToLower(name)
		if lower == "content-length" && body != nil {
			head.WriteString(name + ": " + strconv.Itoa(len(body)) + "\r\n")
			delete(values, lower)
			wroteLength = true
			continue
		}
		if value, ok := values[lower]; ok {
			head.WriteString(name + ": " + value + "\r\n")
			delete(values, lower)
		}
	}

	extraKeys := make([]string, 0, len(values))
	for k := range values {
		extraKeys = append(extraKeys, k)
	}
	sort.Strings(extraKeys)
	for _, name := range extraKeys {
		head.WriteString(name + ": " + values[name] + "\r\n")
	}

	head.WriteString("Connection: keep-alive\r\n")
	if !hasUserAgent {
		head.WriteString("User-Agent: Bun/1.4.3\r\n")
	}
	if !hasAccept {
		head.WriteString("Accept: */*\r\n")
	}
	if req.URL.Scheme == "https" {
		host = strings.TrimSuffix(host, ":443")
	} else {
		host = strings.TrimSuffix(host, ":80")
	}
	head.WriteString("Host: " + host + "\r\n")
	if !hasEncoding {
		head.WriteString("Accept-Encoding: gzip, deflate, br, zstd\r\n")
	}
	if body != nil && !wroteLength {
		head.WriteString("Content-Length: " + strconv.Itoa(len(body)) + "\r\n")
	}
	head.WriteString("\r\n")
	return []byte(head.String()), !hasEncoding, nil
}

func validBunHeaderField(name, value string) bool {
	if name == "" {
		return false
	}
	for i := 0; i < len(name); i++ {
		if b := name[i]; b <= ' ' || b >= 0x7f || strings.IndexByte(`"(),/:;<=>?@[\]{}`, b) >= 0 {
			return false
		}
	}
	for i := 0; i < len(value); i++ {
		if b := value[i]; (b < ' ' && b != '\t') || b == 0x7f {
			return false
		}
	}
	return true
}

// ============================================================================
// Relay Transport Cache & HTTP Handlers
// ============================================================================

func adapterIdentity(proxy C.Proxy) uintptr {
	if proxy == nil {
		return 0
	}
	return 1
}

func getRelayRoundTripper(proxyName string, proxy C.Proxy, fingerprintMode string, skipVerify bool) http.RoundTripper {
	// Resolve current proxy name + underlying proxy server address so reloaded proxies never reuse stale transports
	identityKey := proxyName
	if proxy != nil {
		identityKey = proxyName + "|" + proxy.Addr()
	}
	key := relayTransportKey{
		proxyName:       identityKey,
		adapterPtr:      adapterIdentity(proxy),
		fingerprintMode: fingerprintMode,
		skipVerify:      skipVerify,
	}
	if cached, ok := relayTransportCache.Load(key); ok {
		return cached.(http.RoundTripper)
	}

	var rt http.RoundTripper
	if fingerprintMode == "standard" {
		rt = &http.Transport{
			ForceAttemptHTTP2:     true,
			MaxIdleConns:          64,
			MaxIdleConnsPerHost:   16,
			IdleConnTimeout:       30 * time.Second,
			TLSHandshakeTimeout:   14 * time.Second,
			ResponseHeaderTimeout: 45 * time.Second,
			DialContext: func(ctx context.Context, network string, addr string) (net.Conn, error) {
				host, port, err := splitAddrDefaultPort(addr, "80")
				if err != nil {
					return nil, err
				}
				return dialProxyRawConn(ctx, proxy, host, port)
			},
			DialTLSContext: func(ctx context.Context, network string, addr string) (net.Conn, error) {
				host, port, err := splitAddrDefaultPort(addr, "443")
				if err != nil {
					return nil, err
				}
				rawConn, err := dialProxyRawConn(ctx, proxy, host, port)
				if err != nil {
					return nil, err
				}
				stdConn := tls.Client(rawConn, &tls.Config{
					ServerName:         host,
					InsecureSkipVerify: skipVerify,
					NextProtos:         []string{"h2", "http/1.1"},
				})
				if err := stdConn.HandshakeContext(ctx); err != nil {
					_ = rawConn.Close()
					return nil, err
				}
				return stdConn, nil
			},
		}
	} else {
		// Default / "opencode": full BunTransport (uTLS BoringSSL + HTTP/1.1 WebKit header layout + idle Peek watcher)
		rt = newBunRelayTransport(proxy, skipVerify)
	}

	actual, _ := relayTransportCache.LoadOrStore(key, rt)
	return actual.(http.RoundTripper)
}

func validateTargetURL(raw string, allowLoopback bool) (*url.URL, error) {
	u, err := url.Parse(strings.TrimSpace(raw))
	if err != nil {
		return nil, fmt.Errorf("invalid target URL: %w", err)
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return nil, fmt.Errorf("unsupported scheme %q (must be http or https)", u.Scheme)
	}
	host := u.Hostname()
	if host == "" {
		return nil, fmt.Errorf("missing target host")
	}
	if !allowLoopback {
		if strings.EqualFold(host, "localhost") {
			return nil, fmt.Errorf("loopback target host is disallowed")
		}
		if ip, err := netip.ParseAddr(host); err == nil {
			if ip.IsLoopback() || ip.IsUnspecified() || ip.IsLinkLocalUnicast() {
				return nil, fmt.Errorf("disallowed target address %s", ip.String())
			}
		}
	}
	return u, nil
}

var hopByHopHeaders = map[string]struct{}{
	"connection":          {},
	"proxy-connection":    {},
	"keep-alive":          {},
	"proxy-authenticate":  {},
	"proxy-authorization": {},
	"te":                  {},
	"trailer":             {},
	"transfer-encoding":   {},
	"upgrade":             {},
}

func shouldStripRequestHeader(name string) bool {
	lower := strings.ToLower(name)
	if lower == "authorization" || lower == "host" || lower == "content-length" {
		return true
	}
	if strings.HasPrefix(lower, "x-tesie-") || strings.HasPrefix(lower, "x-tesroute-") {
		return true
	}
	_, hop := hopByHopHeaders[lower]
	return hop
}

func classifyDialFailurePhase(err error) string {
	if err == nil {
		return "unknown"
	}
	msg := strings.ToLower(err.Error())
	switch {
	case errors.Is(err, context.DeadlineExceeded) || strings.Contains(msg, "timeout") || strings.Contains(msg, "deadline exceeded"):
		return "timeout"
	case strings.Contains(msg, "x509:") || strings.Contains(msg, "certificate") || strings.Contains(msg, "tls:"):
		return "tls_verify_failed"
	case errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || strings.Contains(msg, ": eof") || strings.HasSuffix(msg, "eof"):
		return "eof"
	case strings.Contains(msg, "socks") || strings.Contains(msg, "proxyconnect") || strings.Contains(msg, "authentication failed"):
		return "proxy_connect_failed"
	case strings.Contains(msg, "connection refused") || strings.Contains(msg, "forcibly closed") || strings.Contains(msg, "connection reset") || strings.Contains(msg, "unreachable"):
		return "connection_reset"
	default:
		return "dial_failed"
	}
}

func tesrouteRouter() http.Handler {
	r := chi.NewRouter()
	r.Get("/health", tesrouteHealth)
	r.Get("/egress/{name}", tesrouteEgressProbe)
	r.Post("/relay", tesrouteRelay)
	return r
}

func tesieHeader(r *http.Request, tesieKey, legacyKey string) string {
	if v := r.Header.Get(tesieKey); v != "" {
		return v
	}
	return r.Header.Get(legacyKey)
}

func tesrouteHealth(w http.ResponseWriter, r *http.Request) {
	proxies := tunnel.Proxies()
	render.JSON(w, r, render.M{
		"status":          "ok",
		"engine":          "TesieCore",
		"ja4_cipher_hash": opencodeJA4CipherHash,
		"proxies_loaded":  len(proxies),
	})
}

func tesrouteRelay(w http.ResponseWriter, r *http.Request) {
	targetRaw := tesieHeader(r, "X-Tesie-Target-URL", "X-TesRoute-Target-URL")
	if strings.TrimSpace(targetRaw) == "" {
		render.Status(r, http.StatusBadRequest)
		render.JSON(w, r, render.M{"error": "missing X-Tesie-Target-URL"})
		return
	}

	allowLoopback := tesieHeader(r, "X-Tesie-Allow-Loopback", "X-TesRoute-Allow-Loopback") == "1"
	skipVerify := allowLoopback && tesieHeader(r, "X-Tesie-Insecure-Skip-Verify", "X-TesRoute-Insecure-Skip-Verify") == "1"

	targetURL, err := validateTargetURL(targetRaw, allowLoopback)
	if err != nil {
		render.Status(r, http.StatusBadRequest)
		render.JSON(w, r, render.M{"error": err.Error()})
		return
	}

	proxyName := strings.TrimSpace(tesieHeader(r, "X-Tesie-Proxy", "X-TesRoute-Proxy"))
	if proxyName == "" {
		proxyName = "DIRECT"
	}
	adapterProxy, ok := resolveProxyAdapter(proxyName)
	if !ok {
		render.Status(r, http.StatusNotFound)
		render.JSON(w, r, render.M{"error": fmt.Sprintf("proxy %q not found in TesieCore", proxyName)})
		return
	}

	method := strings.ToUpper(strings.TrimSpace(tesieHeader(r, "X-Tesie-Method", "X-TesRoute-Method")))
	if method == "" {
		method = http.MethodPost
	}

	fingerprintMode := strings.ToLower(strings.TrimSpace(tesieHeader(r, "X-Tesie-Fingerprint", "X-TesRoute-Fingerprint")))
	if fingerprintMode == "" {
		fingerprintMode = "opencode"
	}

	timeout := 120 * time.Second
	if rawTimeout := strings.TrimSpace(tesieHeader(r, "X-Tesie-Timeout-Ms", "X-TesRoute-Timeout-Ms")); rawTimeout != "" {
		if ms, err := strconv.ParseInt(rawTimeout, 10, 64); err == nil && ms > 0 && ms <= 600_000 {
			timeout = time.Duration(ms) * time.Millisecond
		}
	}

	var bodyBytes []byte
	if r.Body != nil && r.Body != http.NoBody && method != http.MethodGet && method != http.MethodHead {
		bodyBytes, err = io.ReadAll(r.Body)
		_ = r.Body.Close()
		if err != nil {
			render.Status(r, http.StatusBadRequest)
			render.JSON(w, r, render.M{"error": fmt.Sprintf("read relay request body: %v", err)})
			return
		}
	}

	ctx, cancel := context.WithTimeout(r.Context(), timeout)
	defer cancel()

	var bodyReader io.Reader
	if bodyBytes != nil {
		bodyReader = bytes.NewReader(bodyBytes)
	}
	upReq, err := http.NewRequestWithContext(ctx, method, targetURL.String(), bodyReader)
	if err != nil {
		render.Status(r, http.StatusBadRequest)
		render.JSON(w, r, render.M{"error": err.Error()})
		return
	}
	if bodyBytes != nil {
		upReq.ContentLength = int64(len(bodyBytes))
		upReq.GetBody = func() (io.ReadCloser, error) {
			return io.NopCloser(bytes.NewReader(bodyBytes)), nil
		}
	}

	// Preserve exact incoming header names in map keys so BunTransport sees caller spelling/lowercasing
	for k, vals := range r.Header {
		if shouldStripRequestHeader(k) {
			continue
		}
		for _, v := range vals {
			upReq.Header[k] = append(upReq.Header[k], v)
		}
	}
	if upAuth := tesieHeader(r, "X-Tesie-Upstream-Authorization", "X-TesRoute-Upstream-Authorization"); upAuth != "" {
		upReq.Header.Set("Authorization", upAuth)
	}

	rt := getRelayRoundTripper(proxyName, adapterProxy, fingerprintMode, skipVerify)
	client := &http.Client{
		Transport: rt,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}

	upResp, err := client.Do(upReq)
	if err != nil {
		phase := classifyDialFailurePhase(err)
		render.Status(r, http.StatusBadGateway)
		render.JSON(w, r, render.M{
			"error":  "upstream_dial_failed",
			"phase":  phase,
			"proxy":  proxyName,
			"detail": fmt.Sprintf("%s %q: %v", method, targetURL.String(), err),
		})
		return
	}
	defer upResp.Body.Close()

	for k, vals := range upResp.Header {
		if _, hop := hopByHopHeaders[strings.ToLower(k)]; hop {
			continue
		}
		for _, v := range vals {
			w.Header().Add(k, v)
		}
	}
	w.Header().Set("X-Tesie-Proxy-Used", proxyName)
	w.Header().Set("X-TesRoute-Proxy-Used", proxyName)
	w.WriteHeader(upResp.StatusCode)

	flusher, canFlush := w.(http.Flusher)
	buf := make([]byte, 16*1024)
	for {
		n, readErr := upResp.Body.Read(buf)
		if n > 0 {
			if _, writeErr := w.Write(buf[:n]); writeErr != nil {
				return
			}
			if canFlush {
				flusher.Flush()
			}
		}
		if readErr != nil {
			return
		}
	}
}

func tesrouteEgressProbe(w http.ResponseWriter, r *http.Request) {
	proxyName := chi.URLParam(r, "name")
	if decoded, err := url.PathUnescape(proxyName); err == nil {
		proxyName = decoded
	}
	adapterProxy, ok := resolveProxyAdapter(proxyName)
	if !ok {
		render.Status(r, http.StatusNotFound)
		render.JSON(w, r, render.M{"error": fmt.Sprintf("proxy %q not found", proxyName)})
		return
	}

	probeURLRaw := strings.TrimSpace(r.URL.Query().Get("url"))
	if probeURLRaw == "" {
		probeURLRaw = defaultTraceURL
	}
	allowLoopback := r.URL.Query().Get("allow_loopback") == "1"
	skipVerify := allowLoopback && r.URL.Query().Get("insecure_skip_verify") == "1"

	probeURL, err := validateTargetURL(probeURLRaw, allowLoopback)
	if err != nil {
		render.Status(r, http.StatusBadRequest)
		render.JSON(w, r, render.M{"error": err.Error()})
		return
	}

	timeout := 12 * time.Second
	if rawTimeout := strings.TrimSpace(r.URL.Query().Get("timeout")); rawTimeout != "" {
		if ms, err := strconv.ParseInt(rawTimeout, 10, 64); err == nil && ms > 0 && ms <= 60_000 {
			timeout = time.Duration(ms) * time.Millisecond
		}
	}

	ctx, cancel := context.WithTimeout(r.Context(), timeout)
	defer cancel()

	start := time.Now()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, probeURL.String(), nil)
	if err != nil {
		render.Status(r, http.StatusBadRequest)
		render.JSON(w, r, render.M{"error": err.Error()})
		return
	}
	req.Header.Set("User-Agent", "opencode/latest/2.1.1/cli")
	req.Header.Set("Accept", "*/*")

	rt := getRelayRoundTripper(proxyName, adapterProxy, "opencode", skipVerify)
	client := &http.Client{Transport: rt}
	resp, err := client.Do(req)
	if err != nil {
		render.Status(r, http.StatusBadGateway)
		render.JSON(w, r, render.M{
			"error":  "probe_failed",
			"phase":  classifyDialFailurePhase(err),
			"proxy":  proxyName,
			"detail": err.Error(),
		})
		return
	}
	defer resp.Body.Close()

	bodyBytes, err := io.ReadAll(io.LimitReader(resp.Body, 8192))
	if err != nil {
		render.Status(r, http.StatusBadGateway)
		render.JSON(w, r, render.M{"error": "probe_read_failed", "detail": err.Error()})
		return
	}
	delayMs := time.Since(start).Milliseconds()

	ip, err := parseEgressIP(bodyBytes)
	if err != nil {
		render.Status(r, http.StatusBadGateway)
		render.JSON(w, r, render.M{
			"error":  "probe_parse_failed",
			"proxy":  proxyName,
			"status": resp.StatusCode,
			"detail": err.Error(),
		})
		return
	}

	render.JSON(w, r, render.M{
		"proxy":     proxyName,
		"egress_ip": ip,
		"delay_ms":  delayMs,
	})
}

func parseEgressIP(body []byte) (string, error) {
	text := strings.TrimSpace(string(body))
	if text == "" {
		return "", fmt.Errorf("empty probe response")
	}

	// 1. Cloudflare /cdn-cgi/trace format (`ip=...`)
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "ip=") {
			candidate := strings.TrimSpace(strings.TrimPrefix(line, "ip="))
			if addr, err := netip.ParseAddr(candidate); err == nil {
				return addr.String(), nil
			}
		}
	}

	// 2. JSON `{ "ip": "..." }` or `{ "origin": "..." }`
	if strings.HasPrefix(text, "{") {
		var obj map[string]any
		if err := json.Unmarshal(body, &obj); err == nil {
			for _, key := range []string{"ip", "origin", "address", "query"} {
				if rawVal, ok := obj[key].(string); ok {
					first := strings.TrimSpace(strings.Split(rawVal, ",")[0])
					if addr, err := netip.ParseAddr(first); err == nil {
						return addr.String(), nil
					}
				}
			}
		}
	}

	// 3. Plain-text IP
	firstToken := strings.Fields(text)
	if len(firstToken) > 0 {
		if addr, err := netip.ParseAddr(strings.TrimSpace(firstToken[0])); err == nil {
			return addr.String(), nil
		}
	}

	// 4. Fallback regex scan for IPv4
	if match := ipv4FallbackRegex.FindString(text); match != "" {
		if addr, err := netip.ParseAddr(match); err == nil {
			return addr.String(), nil
		}
	}

	return "", fmt.Errorf("unable to extract valid IP from probe payload")
}
