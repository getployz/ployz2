// ployz-tailcat is a private transport adapter. Stdout is exclusively RPC bytes.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"

	"github.com/tailscale/tailcat"
	"tailscale.com/envknob"
	"tailscale.com/types/logger"
)

const (
	rpcPort         = 7443
	rpcSocket       = "/run/ployz/ployz.sock"
	defaultState    = "/var/lib/ployz/tailcat/state.json"
	capabilityLimit = 16 * 1024
)

var errCapability = errors.New("invalid Tailcat capability")

type endpointState struct {
	Key        *tailcat.PrivateKey `json:"key"`
	Capability tailcat.Addr        `json:"capability"`
}

func main() {
	// ponytail: direct large RPCs are unqualified on Cloud; enable UDP only after qualification.
	envknob.Setenv("TS_DEBUG_ALWAYS_USE_DERP", "true")
	// Upstream debug paths also use the process logger; never print capabilities.
	log.SetOutput(io.Discard)
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := run(ctx, os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run(ctx context.Context, args []string) error {
	if len(args) == 1 && args[0] == "connect" {
		return connect(ctx)
	}
	if len(args) == 1 && args[0] == "serve" {
		return serve(ctx, defaultState)
	}
	if len(args) == 3 && args[0] == "serve" && args[1] == "--state" {
		return serve(ctx, args[2])
	}
	return errors.New("usage: ployz-tailcat connect | serve [--state PATH]")
}

func readCapability(r *bufio.Reader) (tailcat.Addr, error) {
	line, err := r.ReadSlice('\n')
	if err != nil || len(line) > capabilityLimit || len(line) < 2 {
		return "", errCapability
	}
	addr := tailcat.Addr(line[:len(line)-1])
	ci, err := tailcat.ParseAddr(addr)
	if err != nil || ci.PresharedKey.IsZero() || ci.ServerPublic.IsZero() || ci.ServerDiscoPublic.IsZero() {
		return "", errCapability
	}
	return addr, nil
}

func connect(ctx context.Context) error {
	stop := context.AfterFunc(ctx, func() { os.Stdin.Close() })
	defer stop()
	input := bufio.NewReaderSize(os.Stdin, capabilityLimit+1)
	addr, err := readCapability(input)
	if err != nil {
		return err
	}
	client := &tailcat.Client{Server: addr, Logf: logger.Discard}
	defer client.Close()
	dialCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
	conn, err := client.DialTCPPort(dialCtx, rpcPort)
	cancel()
	if err != nil {
		return errors.New("Tailcat connection failed")
	}
	defer conn.Close()
	// Closing stdin releases the upload goroutine when the remote ends first.
	defer os.Stdin.Close()
	if err := relay(ctx, conn, input, os.Stdout); err != nil {
		return err
	}
	drainCtx, drainCancel := context.WithTimeout(ctx, 5*time.Second)
	defer drainCancel()
	if err := client.DrainTCP(drainCtx); err != nil {
		return errors.New("Tailcat connection drain failed")
	}
	return nil
}

func relay(ctx context.Context, conn net.Conn, input io.Reader, output io.Writer) error {
	upload := make(chan error, 1)
	download := make(chan error, 1)
	go func() {
		_, err := io.Copy(conn, input)
		if err == nil {
			if half, ok := conn.(interface{ CloseWrite() error }); ok {
				err = half.CloseWrite()
			} else {
				err = errors.New("missing half-close")
			}
		}
		upload <- err
	}()
	go func() { _, err := io.Copy(output, conn); download <- err }()
	for {
		select {
		case <-ctx.Done():
			conn.Close()
			return errors.New("Tailcat connection canceled")
		case err := <-upload:
			if err != nil {
				conn.Close()
				return errors.New("Tailcat upload failed")
			}
			upload = nil
		case err := <-download:
			if err != nil {
				conn.Close()
				return errors.New("Tailcat download failed")
			}
			return nil
		}
	}
}

func readState(path string) (*endpointState, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() || info.Mode().Perm()&0077 != 0 || info.Size() > 64*1024 {
		return nil, errors.New("state must be a private regular file")
	}
	dir, err := os.Stat(filepath.Dir(path))
	if err != nil {
		return nil, err
	}
	if dir.Mode().Perm()&0077 != 0 {
		return nil, errors.New("state directory must be private")
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	var state endpointState
	decoder := json.NewDecoder(io.LimitReader(f, 64*1024))
	if err := decoder.Decode(&state); err != nil {
		return nil, errors.New("invalid Tailcat state")
	}
	if decoder.Decode(new(any)) != io.EOF {
		return nil, errors.New("invalid Tailcat state")
	}
	if state.Key == nil || state.Key.Private.IsZero() || state.Key.Public.PresharedKey.IsZero() {
		return nil, errors.New("invalid Tailcat state")
	}
	return &state, nil
}

func writeState(path string, state *endpointState) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	info, err := os.Stat(dir)
	if err != nil {
		return err
	}
	if info.Mode().Perm()&0077 != 0 {
		return errors.New("state directory must be private")
	}
	data, err := json.Marshal(state)
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(dir, ".tailcat-*")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if _, err := f.Write(data); err != nil {
		f.Close()
		return err
	}
	if err := f.Sync(); err != nil {
		f.Close()
		return err
	}
	if err := f.Close(); err != nil {
		return err
	}
	if err := os.Rename(f.Name(), path); err != nil {
		return err
	}
	d, err := os.Open(dir)
	if err != nil {
		return err
	}
	defer d.Close()
	return d.Sync()
}

func serve(ctx context.Context, path string) error {
	state, err := readState(path)
	if errors.Is(err, os.ErrNotExist) {
		state = &endpointState{Key: tailcat.NewPrivateKey()}
		// Save identity before exposing an endpoint; restart must never silently rotate it.
		if err = writeState(path, state); err != nil {
			return errors.New("cannot persist Tailcat identity")
		}
	} else if err != nil {
		return errors.New("cannot read Tailcat identity")
	}
	server := &tailcat.Server{
		Key: state.Key.Private, PresharedKey: state.Key.Public.PresharedKey,
		Logf: logger.Discard, MaxClients: 128, ClientIdleTimeout: 30 * time.Second,
		OnTCP: func(port uint16) func(net.Conn) {
			if port != rpcPort {
				return nil
			}
			return func(remote net.Conn) {
				local, err := net.DialTimeout("unix", rpcSocket, 5*time.Second)
				if err != nil {
					remote.Close()
					return
				}
				tailcat.ProxyConns(remote, local)
			}
		},
	}
	// Preserve the selected bootstrap relay, as well as keys, across restarts.
	if len(state.Key.Public.Region) > 0 {
		server.Region = state.Key.Public.Region[0]
	} else {
		server.RegionID = state.Key.Public.RegionID
	}
	if err := server.Start(); err != nil {
		server.Close()
		return errors.New("Tailcat endpoint startup failed")
	}
	defer server.Close()
	state.Capability = server.TailcatAddr()
	state.Key.Public, err = tailcat.ParseAddr(state.Capability)
	if err != nil {
		return errors.New("Tailcat endpoint address failed")
	}
	if err := writeState(path, state); err != nil {
		return errors.New("cannot persist Tailcat endpoint")
	}
	<-ctx.Done()
	return nil
}
