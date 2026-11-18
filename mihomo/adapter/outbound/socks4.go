package outbound

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"strconv"

	N "github.com/metacubex/mihomo/common/net"
	"github.com/metacubex/mihomo/component/ca"
	"github.com/metacubex/mihomo/component/resolver"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/transport/socks4"

	"github.com/metacubex/tls"
)

type Socks4 struct {
	*Base
	option    *Socks4Option
	user      string
	remoteDNS bool
	tls       bool
	tlsConfig *tls.Config
}

type Socks4Option struct {
	BasicOption
	Name           string `proxy:"name"`
	Server         string `proxy:"server"`
	Port           int    `proxy:"port"`
	UserName       string `proxy:"username,omitempty"`
	RemoteDNS      bool   `proxy:"remote-dns,omitempty"`
	TLS            bool   `proxy:"tls,omitempty"`
	SkipCertVerify bool   `proxy:"skip-cert-verify,omitempty"`
	NameCertVerify string `proxy:"name-cert-verify,omitempty"`
	Fingerprint    string `proxy:"fingerprint,omitempty"`
	Certificate    string `proxy:"certificate,omitempty"`
	PrivateKey     string `proxy:"private-key,omitempty"`
}

func resolveIPv4Host(ctx context.Context, host string) (netip.Addr, error) {
	if ip, err := netip.ParseAddr(host); err == nil {
		ip = ip.Unmap()
		if ip.Is4() {
			return ip, nil
		}
		return netip.Addr{}, fmt.Errorf("socks4 does not support IPv6 target %s", host)
	}
	if resolver.DefaultResolver != nil || resolver.SystemResolver != nil {
		if ip, err := resolver.ResolveIPv4(ctx, host); err == nil && ip.IsValid() {
			u := ip.Unmap()
			if u.Is4() {
				return u, nil
			}
		}
	}
	addrs, err := net.DefaultResolver.LookupNetIP(ctx, "ip4", host)
	if err != nil {
		return netip.Addr{}, err
	}
	for _, a := range addrs {
		u := a.Unmap()
		if u.Is4() {
			return u, nil
		}
	}
	return netip.Addr{}, fmt.Errorf("no IPv4 address found for %s", host)
}

func (ss *Socks4) targetAddress(ctx context.Context, metadata *C.Metadata, forceResolve bool) (string, error) {
	port := strconv.FormatUint(uint64(metadata.DstPort), 10)
	if metadata.DstIP.IsValid() {
		u := metadata.DstIP.Unmap()
		if u.Is4() {
			return net.JoinHostPort(u.String(), port), nil
		}
		if metadata.Host == "" {
			return "", fmt.Errorf("socks4 does not support IPv6 target %s", metadata.DstIP.String())
		}
	}
	if !ss.remoteDNS || forceResolve {
		ip4, err := resolveIPv4Host(ctx, metadata.Host)
		if err != nil {
			return "", fmt.Errorf("socks4 resolve %s: %w", metadata.Host, err)
		}
		return net.JoinHostPort(ip4.String(), port), nil
	}
	return metadata.RemoteAddress(), nil
}

// StreamConnContext implements C.ProxyAdapter (resolves IPv4 locally for pure SOCKS4, or uses SOCKS4a when remote-dns is enabled)
func (ss *Socks4) StreamConnContext(ctx context.Context, c net.Conn, metadata *C.Metadata) (net.Conn, error) {
	if ss.tls {
		cc := tls.Client(c, ss.tlsConfig)
		if err := cc.HandshakeContext(ctx); err != nil {
			return nil, fmt.Errorf("%s connect error: %w", ss.addr, err)
		}
		c = cc
	}

	targetAddr, err := ss.targetAddress(ctx, metadata, false)
	if err != nil {
		return nil, err
	}

	if err := ss.clientHandshakeContext(ctx, c, targetAddr, socks4.CmdConnect, ss.user); err != nil {
		return nil, err
	}
	return c, nil
}

// DialContext implements C.ProxyAdapter
func (ss *Socks4) DialContext(ctx context.Context, metadata *C.Metadata) (_ C.Conn, err error) {
	c, err := ss.dialer.DialContext(ctx, "tcp", ss.addr)
	if err != nil {
		return nil, fmt.Errorf("%s connect error: %w", ss.addr, err)
	}

	c, err = ss.StreamConnContext(ctx, c, metadata)
	if err != nil {
		_ = c.Close()
		// If SOCKS4a remote-dns was rejected by a strict SOCKS4 server, transparently retry once with resolved IPv4
		if ss.remoteDNS && !metadata.DstIP.IsValid() && metadata.Host != "" && errors.Is(err, socks4.ErrRequestRejected) {
			c2, dialErr := ss.dialer.DialContext(ctx, "tcp", ss.addr)
			if dialErr == nil {
				targetAddr, resErr := ss.targetAddress(ctx, metadata, true)
				if resErr == nil {
					if hsErr := ss.clientHandshakeContext(ctx, c2, targetAddr, socks4.CmdConnect, ss.user); hsErr == nil {
						return NewConn(c2, ss), nil
					}
				}
				_ = c2.Close()
			}
		}
		return nil, err
	}

	return NewConn(c, ss), nil
}

// ProxyInfo implements C.ProxyAdapter
func (ss *Socks4) ProxyInfo() C.ProxyInfo {
	info := ss.Base.ProxyInfo()
	info.DialerProxy = ss.option.DialerProxy
	return info
}

func (ss *Socks4) clientHandshakeContext(ctx context.Context, c net.Conn, addr string, command socks4.Command, user string) (err error) {
	if ctx.Done() != nil {
		done := N.SetupContextForConn(ctx, c)
		defer done(&err)
	}
	return socks4.ClientHandshake(c, addr, command, user)
}

func NewSocks4(option Socks4Option) (*Socks4, error) {
	var tlsConfig *tls.Config
	if option.TLS {
		var err error
		tlsConfig, err = ca.GetTLSConfig(ca.Option{
			TLSConfig: &tls.Config{
				InsecureSkipVerify: option.SkipCertVerify,
				ServerName:         option.Server,
			},
			Fingerprint:    option.Fingerprint,
			NameCertVerify: option.NameCertVerify,
			Certificate:    option.Certificate,
			PrivateKey:     option.PrivateKey,
		})
		if err != nil {
			return nil, err
		}
	}

	outbound := &Socks4{
		Base: NewBase(BaseOption{
			Name:         option.Name,
			Addr:         net.JoinHostPort(option.Server, strconv.Itoa(option.Port)),
			Type:         C.Socks4,
			ProviderName: option.ProviderName,
			UDP:          false,
			TFO:          option.TFO,
			MPTCP:        option.MPTCP,
			Interface:    option.Interface,
			RoutingMark:  option.RoutingMark,
			Prefer:       option.IPVersion,
		}),
		option:    &option,
		user:      option.UserName,
		remoteDNS: option.RemoteDNS,
		tls:       option.TLS,
		tlsConfig: tlsConfig,
	}
	outbound.dialer = option.NewDialer(outbound.DialOptions())
	return outbound, nil
}
