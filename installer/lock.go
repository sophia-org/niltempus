package main

import (
	"fmt"
	"os"
	"path/filepath"
	"syscall"
)

func buildLock(loc Locations) (func(), error) {
	if err := os.MkdirAll(loc.Cache, 0700); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(filepath.Join(loc.Cache, "build.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		f.Close()
		return nil, fmt.Errorf("another desktop build owns the caches: %w", err)
	}
	return func() { f.Close() }, nil
}
