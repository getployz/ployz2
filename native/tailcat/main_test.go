package main

import (
	"bufio"
	"bytes"
	"context"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/tailscale/tailcat"
	"tailscale.com/envknob"
)

func TestCapabilityFraming(t *testing.T) {
	key := tailcat.NewPrivateKey()
	key.Public.RegionID = 1
	capability := string(key.Public.Addr())
	reader := bufio.NewReaderSize(strings.NewReader(capability+"\nRPC\nbytes"), capabilityLimit+1)
	got, err := readCapability(reader)
	if err != nil || string(got) != capability {
		t.Fatal("valid capability rejected")
	}
	remaining, err := io.ReadAll(reader)
	if err != nil || string(remaining) != "RPC\nbytes" {
		t.Fatal("RPC bytes consumed by capability parser")
	}
	noPSK := key.Public
	noPSK.PresharedKey = tailcat.PresharedKey{}
	for _, input := range []string{"", "\n", capability, "secret-malformed\n", strings.Repeat("x", capabilityLimit) + "\n", string(noPSK.Addr()) + "\n"} {
		_, err := readCapability(bufio.NewReaderSize(strings.NewReader(input), capabilityLimit+1))
		if err != errCapability {
			t.Fatal("invalid capability did not produce the fixed redacted error")
		}
	}
}

func TestStateIsPrivateAndStable(t *testing.T) {
	path := filepath.Join(t.TempDir(), "state", "tailcat.json")
	state := &endpointState{Key: tailcat.NewPrivateKey()}
	if err := writeState(path, state); err != nil {
		t.Fatal(err)
	}
	first, err := readState(path)
	if err != nil {
		t.Fatal(err)
	}
	if !first.Key.Private.Equal(state.Key.Private) || first.Key.Public.PresharedKey != state.Key.Public.PresharedKey {
		t.Fatal("identity changed")
	}
	if err := writeState(path, first); err != nil {
		t.Fatal(err)
	}
	second, err := readState(path)
	if err != nil || !second.Key.Private.Equal(first.Key.Private) {
		t.Fatal("identity changed on rewrite")
	}
	if err := os.Chmod(path, 0644); err != nil {
		t.Fatal(err)
	}
	if _, err := readState(path); err == nil {
		t.Fatal("public state accepted")
	}
	link := path + ".link"
	if err := os.Symlink(path, link); err != nil {
		t.Fatal(err)
	}
	if _, err := readState(link); err == nil {
		t.Fatal("symlink state accepted")
	}
	if err := os.Chmod(filepath.Dir(path), 0755); err != nil {
		t.Fatal(err)
	}
	if err := writeState(path, state); err == nil {
		t.Fatal("public state directory accepted")
	}
}

func TestRelayHalfCloseAndCancellation(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	conn, err := net.Dial("tcp", listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	peer, err := listener.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer peer.Close()
	_ = peer.SetDeadline(time.Now().Add(5 * time.Second))
	request := bytes.Repeat([]byte("request"), 10000)
	response := bytes.Repeat([]byte("response"), 10000)
	peerDone := make(chan error, 1)
	go func() {
		got, err := io.ReadAll(peer)
		if err == nil && !bytes.Equal(got, request) {
			err = io.ErrUnexpectedEOF
		}
		if err == nil {
			_, err = peer.Write(response)
		}
		if err == nil {
			err = peer.(*net.TCPConn).CloseWrite()
		}
		peerDone <- err
	}()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	var got bytes.Buffer
	if err := relay(ctx, conn, bytes.NewReader(request), &got); err != nil {
		t.Fatal(err)
	}
	if err := <-peerDone; err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got.Bytes(), response) {
		t.Fatal("response truncated after request half-close")
	}
	a, b := net.Pipe()
	defer a.Close()
	defer b.Close()
	canceled, stop := context.WithCancel(context.Background())
	stop()
	if err := relay(canceled, a, strings.NewReader("request"), io.Discard); err == nil {
		t.Fatal("cancellation succeeded")
	}
}

func TestCorruptStateIsNotReplaced(t *testing.T) {
	path := filepath.Join(t.TempDir(), "state.json")
	original := []byte("secret-malformed-state")
	if err := os.WriteFile(path, original, 0600); err != nil {
		t.Fatal(err)
	}
	if err := serve(context.Background(), path); err == nil || err.Error() != "cannot read Tailcat identity" {
		t.Fatal("corrupt state did not fail with a redacted error")
	}
	got, err := os.ReadFile(path)
	if err != nil || !bytes.Equal(got, original) {
		t.Fatal("corrupt state replaced")
	}
}

// Run production startup in a separate process: Setenv must precede goroutines.
func TestRelayOnlyStartup(t *testing.T) {
	if os.Getenv("PLOYZ_TEST_STARTUP") == "1" {
		os.Args = []string{"ployz-tailcat", "--version"}
		main()
		if !envknob.Bool("TS_DEBUG_ALWAYS_USE_DERP") {
			t.Fatal("management helper enabled direct UDP")
		}
		return
	}
	cmd := exec.Command(os.Args[0], "-test.run=^TestRelayOnlyStartup$")
	cmd.Env = append(os.Environ(), "PLOYZ_TEST_STARTUP=1", "TS_DEBUG_ALWAYS_USE_DERP=false")
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("helper startup: %v: %s", err, output)
	}
}
