//go:build !windows

package trailers

import (
	"os"
	"os/exec"
)

func prepareCmd(cmd *exec.Cmd) {
	// No special preparation needed
}

func interruptProcess(p *os.Process) error {
	return p.Signal(os.Interrupt)
}
