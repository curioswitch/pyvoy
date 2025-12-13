//go:build windows

package trailers

import (
	"os"
	"os/exec"
	"syscall"
)

func prepareCmd(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{
		CreationFlags: syscall.CREATE_NEW_PROCESS_GROUP,
	}
}

func interruptProcess(p *os.Process) error {
	const CTRL_BREAK_EVENT = 1
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
