package route

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"io"
	"net"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/metacubex/http"
	"github.com/metacubex/http/httptest"
)

func isGREASEValue(v uint16) bool {
	return (v & 0x0f0f) == 0x0a0a
}

type parsedClientHello struct {
	ciphers    []uint16
	extensions []string
	alpn       []string
}

func parseClientHelloRecord(record []byte) (*parsedClientHello, error) {
	if len(record) < 9 || record[0] != 0x16 || record[5] != 0x01 {
		return nil, fmt.Errorf("expected TLS ClientHello record, got len=%d", len(record))
	}
	pos := 9 + 34
	if len(record) < pos+1 {
		return nil, fmt.Errorf("ClientHello truncated before session_id")
	}
	sessionIDLen := int(record[pos])
	pos += 1 + sessionIDLen
	if len(record) < pos+2 {
		return nil, fmt.Errorf("ClientHello truncated before cipher_suites")
	}
	cipherLen := int(binary.BigEndian.Uint16(record[pos : pos+2]))
	pos += 2
	if cipherLen%2 != 0 || len(record) < pos+cipherLen {
		return nil, fmt.Errorf("invalid cipher_suites length %d", cipherLen)
	}
	ciphers := make([]uint16, 0, cipherLen/2)
	for i := 0; i < cipherLen; i += 2 {
		ciphers = append(ciphers, binary.BigEndian.Uint16(record[pos+i:pos+i+2]))
	}
	pos += cipherLen

	if len(record) < pos+1 {
		return nil, fmt.Errorf("ClientHello truncated before compression")
	}
	compLen := int(record[pos])
	pos += 1 + compLen
	if len(record) < pos+2 {
		return nil, fmt.Errorf("ClientHello truncated before extensions")
	}
	extTotal := int(binary.BigEndian.Uint16(record[pos : pos+2]))
	pos += 2
	end := pos + extTotal
	if len(record) < end {
		return nil, fmt.Errorf("ClientHello extensions truncated (%d < %d)", len(record), end)
	}

	var exts []string
	var alpn []string
	for pos < end {
		extType := binary.BigEndian.Uint16(record[pos : pos+2])
		extLen := int(binary.BigEndian.Uint16(record[pos+2 : pos+4]))
		if !isGREASEValue(extType) {
			exts = append(exts, fmt.Sprintf("%04x", extType))
		}
		if extType == 0x0010 && extLen >= 2 {
			q := pos + 6
			for q < pos+4+extLen {
				l := int(record[q])
				q++
				alpn = append(alpn, string(record[q:q+l]))
				q += l
			}
		}
		pos += 4 + extLen
	}

	return &parsedClientHello{ciphers: ciphers, extensions: exts, alpn: alpn}, nil
}

func TestTesRouteOpenCodeJA4CipherHash(t *testing.T) {
	clientConn, serverConn := net.Pipe()
	defer clientConn.Close()
	defer serverConn.Close()

	helloBytesCh := make(chan []byte, 1)
	errCh := make(chan error, 1)
	go func() {
		buf := make([]byte, 4096)
		_ = serverConn.SetReadDeadline(time.Now().Add(3 * time.Second))
		n, err := serverConn.Read(buf)
		if err != nil {
			errCh <- err
			return
		}
		helloBytesCh <- append([]byte(nil), buf[:n]...)
		_ = serverConn.Close()
	}()

	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		c, _ := dialOpenCodeUTLS(ctx, clientConn, "opencode.ai", true)
		if c != nil {
			_ = c.Close()
		}
	}()

	var rawHello []byte
	select {
	case rawHello = <-helloBytesCh:
	case readErr := <-errCh:
		t.Fatalf("failed to read ClientHello: %v", readErr)
	case <-time.After(4 * time.Second):
		t.Fatal("timed out waiting for ClientHello")
	}

	parsed, err := parseClientHelloRecord(rawHello)
	if err != nil {
		t.Fatalf("parseClientHelloRecord: %v", err)
	}

	hexCiphers := make([]string, 0, len(parsed.ciphers))
	for _, cs := range parsed.ciphers {
		if isGREASEValue(cs) || cs == 0x00ff {
			continue
		}
		hexCiphers = append(hexCiphers, fmt.Sprintf("%04x", cs))
	}
	sort.Strings(hexCiphers)
	sum := sha256.Sum256([]byte(strings.Join(hexCiphers, ",")))
	gotHash := hex.EncodeToString(sum[:])[:12]

	if gotHash != opencodeJA4CipherHash {
		t.Fatalf("JA4 cipher hash mismatch: got %s (ciphers=%v), want %s", gotHash, hexCiphers, opencodeJA4CipherHash)
	}

	wantExts := "0000,0017,ff01,000a,000b,0023,0010,0005,000d,0012,0033,002d,002b"
	if gotExts := strings.Join(parsed.extensions, ","); gotExts != wantExts {
		t.Fatalf("extension list mismatch: got %s, want %s", gotExts, wantExts)
	}
	if strings.Join(parsed.alpn, ",") != "http/1.1" {
		t.Fatalf("ALPN mismatch: got %v, want [http/1.1]", parsed.alpn)
	}
}

func TestTesRouteBunRequestHeadWireLayout(t *testing.T) {
	req, err := http.NewRequest("POST", "https://opencode.ai/inference/openai/v1/chat/completions", nil)
	if err != nil {
		t.Fatalf("NewRequest: %v", err)
	}
	req.Header["Authorization"] = []string{"Bearer public"}
	req.Header["Content-Type"] = []string{"application/json"}
	req.Header["User-Agent"] = []string{"opencode/latest/2.1.1/cli"}
	req.Header["x-opencode-session"] = []string{"ses_abc123"}
	req.Header["x-opencode-client"] = []string{"cli"}

	headBytes, decompress, err := bunRequestHead(req, []byte(`{"stream":true}`))
	if err != nil {
		t.Fatalf("bunRequestHead: %v", err)
	}
	if !decompress {
		t.Fatalf("expected decompress=true")
	}
	head := string(headBytes)
	wantLines := []string{
		"POST /inference/openai/v1/chat/completions HTTP/1.1",
		"Authorization: Bearer public",
		"Content-Length: 15",
		"Content-Type: application/json",
		"User-Agent: opencode/latest/2.1.1/cli",
		"x-opencode-client: cli",
		"x-opencode-session: ses_abc123",
		"Connection: keep-alive",
		"Accept: */*",
		"Host: opencode.ai",
		"Accept-Encoding: gzip, deflate, br, zstd",
	}
	gotLines := strings.Split(strings.TrimSuffix(head, "\r\n\r\n"), "\r\n")
	if strings.Join(gotLines, "\n") != strings.Join(wantLines, "\n") {
		t.Fatalf("Bun HTTP/1.1 header wire layout mismatch:\ngot:\n%s\n\nwant:\n%s", strings.Join(gotLines, "\n"), strings.Join(wantLines, "\n"))
	}
}

func TestTesRouteRelayStreamingAndAuthIsolation(t *testing.T) {
	upstream := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if got := r.Header.Get("Authorization"); got != "Bearer public" {
			http.Error(w, fmt.Sprintf("unexpected upstream Authorization: %q", got), 401)
			return
		}
		if got := r.Header.Get("X-TesRoute-Target-URL"); got != "" {
			http.Error(w, fmt.Sprintf("leaked X-TesRoute header: %q", got), 400)
			return
		}
		if got := r.Header.Get("x-opencode-session"); got != "ses_test123" {
			http.Error(w, fmt.Sprintf("missing x-opencode-session: %q", got), 400)
			return
		}

		bodyBytes, _ := io.ReadAll(r.Body)
		if !strings.Contains(string(bodyBytes), `"stream":true`) {
			http.Error(w, "missing expected JSON body", 400)
			return
		}

		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(200)
		flusher, _ := w.(http.Flusher)
		for _, frame := range []string{
			"data: {\"chunk\":1}\n\n",
			"data: {\"chunk\":2}\n\n",
			"data: [DONE]\n\n",
		} {
			_, _ = io.WriteString(w, frame)
			if flusher != nil {
				flusher.Flush()
			}
		}
	}))
	defer upstream.Close()

	ctlRouter := router(false, "ctl-secret", "", Cors{})
	req := httptest.NewRequest("POST", "/tesroute/relay", strings.NewReader(`{"stream":true}`))
	req.Header.Set("Authorization", "Bearer ctl-secret")
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-TesRoute-Target-URL", upstream.URL+"/inference/v1/chat/completions")
	req.Header.Set("X-TesRoute-Proxy", "DIRECT")
	req.Header.Set("X-TesRoute-Upstream-Authorization", "Bearer public")
	req.Header.Set("X-TesRoute-Allow-Loopback", "1")
	req.Header.Set("X-TesRoute-Insecure-Skip-Verify", "1")
	req.Header.Set("x-opencode-session", "ses_test123")

	rec := httptest.NewRecorder()
	ctlRouter.ServeHTTP(rec, req)

	if rec.Code != 200 {
		t.Fatalf("expected 200, got %d: %s", rec.Code, rec.Body.String())
	}
	if got := rec.Header().Get("X-TesRoute-Proxy-Used"); got != "DIRECT" {
		t.Fatalf("expected X-TesRoute-Proxy-Used=DIRECT, got %q", got)
	}
	body := rec.Body.String()
	for _, want := range []string{`{"chunk":1}`, `{"chunk":2}`, `[DONE]`} {
		if !strings.Contains(body, want) {
			t.Fatalf("missing streamed SSE chunk %q in body %q", want, body)
		}
	}

	unauthReq := httptest.NewRequest("GET", "/tesroute/health", nil)
	unauthRec := httptest.NewRecorder()
	ctlRouter.ServeHTTP(unauthRec, unauthReq)
	if unauthRec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401 without secret, got %d", unauthRec.Code)
	}
}

func TestTesRouteParseEgressIP(t *testing.T) {
	cases := []struct {
		name string
		body string
		want string
	}{
		{
			name: "cloudflare trace",
			body: "fl=123f45\nh=cloudflare.com\nip=103.35.111.126\nts=1700000000\n",
			want: "103.35.111.126",
		},
		{
			name: "json ip",
			body: `{"ip":"198.51.100.42"}`,
			want: "198.51.100.42",
		},
		{
			name: "httpbin origin",
			body: `{"origin":"203.0.113.9, 10.0.0.1"}`,
			want: "203.0.113.9",
		},
		{
			name: "plain text ipv6",
			body: "2001:db8::1\n",
			want: "2001:db8::1",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, err := parseEgressIP([]byte(tc.body))
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if got != tc.want {
				t.Fatalf("got %q, want %q", got, tc.want)
			}
		})
	}
}
