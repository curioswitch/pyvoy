//go:build windows

package trailers

import (
	"os"
	"os/exec"
	"syscall"
)

// If we terminate the process normally on Windows, the pyvoy CLI will
// close without closing envoy. We need to send a ctrl event instead
// so it shuts down in the same way as receiving ctrl+c from the console.

const CTRL_BREAK_EVENT = 1

func prepareCmd(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{
		CreationFlags: syscall.CREATE_NEW_PROCESS_GROUP,
	}
}

func interruptProcess(p *os.Process) error {
	dll, err := syscall.LoadDLL("kernel32.dll")
	if err != nil {
		return err
	}
	defer dll.Release()
	proc, err := dll.FindProc("GenerateConsoleCtrlEvent")
	if err != nil {
		return err
	}
	r1, _, err := proc.Call(uintptr(CTRL_BREAK_EVENT), uintptr(p.Pid))
	if r1 == 0 {
		return err
	}
	return nil
}
