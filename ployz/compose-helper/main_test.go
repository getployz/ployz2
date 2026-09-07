package main

import (
	"strings"
	"testing"
)

func TestCommandSecretRejectsDriverOptions(t *testing.T) {
	for _, options := range []any{object{"command": "printf two"}, object{}, nil} {
		_, err := secretSource("token", object{"x-command": "printf one", "driver_opts": options})
		if err == nil || !strings.Contains(err.Error(), "x-command cannot be combined with driver or driver_opts") {
			t.Fatalf("accepted conflicting driver_opts or lost diagnostic: %v", err)
		}
	}
}
