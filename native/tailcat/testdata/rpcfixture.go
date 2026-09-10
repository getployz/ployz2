// Local transport fixture for the Rust Machine RPC contract. No root or public relay.
package main

import (
	"fmt"
	"io"
	"log"
	"net"
	"net/http/httptest"
	"os"

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
	server := &tailcat.Server{Region: region, Logf: logger.Discard, OnTCP: func(port uint16) func(net.Conn) {
		if port != 7443 {
			return nil
		}
		return func(remote net.Conn) {
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
	io.Copy(io.Discard, os.Stdin)
}
