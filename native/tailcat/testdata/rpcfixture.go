// Local transport fixture for the Rust Machine RPC contract. No root or public relay.
package main

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"log"
	"net"
	"net/http/httptest"
	"os"
	"sync"
	"time"

	"github.com/tailscale/tailcat"
	"tailscale.com/derp/derpserver"
	"tailscale.com/tailcfg"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
)

func main() {
	log.SetOutput(io.Discard)
	derp := derpserver.New(key.NewNode(), logger.Discard)
	defer derp.Close()
	https := httptest.NewTLSServer(derpserver.Handler(derp))
	defer https.Close()
	region := &tailcfg.DERPRegion{RegionID: 1, RegionCode: "local", Nodes: []*tailcfg.DERPNode{{
		Name: "local", RegionID: 1, HostName: "127.0.0.1", IPv4: "127.0.0.1", IPv6: "none",
		DERPPort: https.Listener.Addr().(*net.TCPAddr).Port, STUNPort: -1, InsecureForTests: true,
	}}}
	var mu sync.Mutex
	seen := map[string]bool{}
	active := 0
	server := &tailcat.Server{Region: region, Logf: logger.Discard, MaxClients: 16, ClientIdleTimeout: 500 * time.Millisecond, OnTCP: func(port uint16) func(net.Conn) {
		if port != 7443 {
			return nil
		}
		return func(remote net.Conn) {
			// The remote IPv6 host is derived from this client's public key.
			host, _, err := net.SplitHostPort(remote.RemoteAddr().String())
			if err != nil {
				panic("invalid client address")
			}
			mu.Lock()
			seen[host] = true
			active++
			mu.Unlock()
			defer func() {
				mu.Lock()
				active--
				mu.Unlock()
			}()
			local, err := net.Dial("unix", os.Args[1])
			if err != nil {
				remote.Close()
				return
			}
			tailcat.ProxyConns(remote, local)
		}
	}}
	if err := server.Start(); err != nil {
		panic("local Tailcat fixture startup failed")
	}
	defer server.Close()
	// Protected pipe to the test; capability never appears in argv or logs.
	fmt.Fprintln(os.Stdout, server.TailcatAddr())
	commands := bufio.NewScanner(os.Stdin)
	for commands.Scan() {
		switch commands.Text() {
		case "state":
			mu.Lock()
			count := active
			mu.Unlock()
			fmt.Fprintf(os.Stdout, "%d %d\n", count, len(server.Status().Peer))
		case "reset-clients":
			mu.Lock()
			clear(seen)
			mu.Unlock()
			fmt.Fprintln(os.Stdout, "reset-ok")
		case "clients":
			mu.Lock()
			fmt.Fprintln(os.Stdout, len(seen))
			mu.Unlock()
		case "churn":
			for range 8 {
				churn(server.TailcatAddr(), false)
				churn(server.TailcatAddr(), true)
			}
			fmt.Fprintln(os.Stdout, "churn-ok")
		default:
			panic("unknown fixture command")
		}
	}
}

func churn(addr tailcat.Addr, invalid bool) {
	budget := 5 * time.Second
	if invalid {
		ci, err := tailcat.ParseAddr(addr)
		if err != nil {
			panic("invalid fixture address")
		}
		ci.PresharedKey = tailcat.NewPresharedKey()
		addr = ci.Addr()
		budget = 300 * time.Millisecond
	}
	client := &tailcat.Client{Server: addr, Logf: logger.Discard}
	defer client.Close()
	ctx, cancel := context.WithTimeout(context.Background(), budget)
	defer cancel()
	conn, err := client.DialTCPPort(ctx, 7443)
	if invalid {
		if err == nil {
			conn.Close()
			panic("invalid PSK connected")
		}
		return
	}
	if err != nil {
		panic("successful churn connection failed")
	}
	conn.Close()
	if err := client.DrainTCP(ctx); err != nil {
		panic("successful churn cleanup failed")
	}
}
