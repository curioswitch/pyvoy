package trailers

import (
	"bufio"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"strconv"
	"strings"
	"testing"

	"github.com/shirou/gopsutil/v4/process"
	"github.com/stretchr/testify/require"
)

func TestHTTP2(t *testing.T) {
	for _, pyinterface := range []string{"asgi", "wsgi"} {
		t.Run(pyinterface, func(t *testing.T) {
			stdoutR, stdoutW := io.Pipe()
			cmd := exec.Command("pyvoy", fmt.Sprintf("tests.apps.%s.kitchensink", pyinterface), "--port", "0", "--interface", pyinterface)
			cmd.Stdout = stdoutW
			cmd.Stderr = stdoutW

			stdout := bufio.NewScanner(stdoutR)

			if err := cmd.Start(); err != nil {
				t.Fatalf("failed to start pyvoy: %v", err)
			}
			defer func() {
				_ = stdoutR.Close()
				_ = stdoutW.Close()
				p, _ := process.NewProcessWithContext(t.Context(), int32(cmd.Process.Pid))
				_ = p.TerminateWithContext(t.Context())
				_ = cmd.Wait()
			}()

			var port int
			for stdout.Scan() {
				l := strings.TrimSpace(stdout.Text())
				if !strings.Contains(l, "pyvoy listening on") {
					continue
				}
				port, _ = strconv.Atoi(l[len("pyvoy listening on 127.0.0.1:"):])
				break
			}

			var prots http.Protocols
			prots.SetUnencryptedHTTP2(true)
			cl := &http.Client{
				Transport: &http.Transport{
					ForceAttemptHTTP2: true,
					Protocols:         &prots,
				},
			}

			if pyinterface == "asgi" {
				t.Run("trailers_only", func(t *testing.T) {
					req, _ := http.NewRequestWithContext(t.Context(), "GET", fmt.Sprintf("http://localhost:%d/trailers-only", port), nil)
					req.Header.Set("TE", "trailers")
					res, err := cl.Do(req)
					require.NoError(t, err)
					defer res.Body.Close()
					require.Equal(t, http.StatusOK, res.StatusCode)
					body, err := io.ReadAll(res.Body)
					require.NoError(t, err)
					require.Empty(t, body)
					require.Equal(t, "last", res.Trailer.Get("X-First"))
					require.Equal(t, "first", res.Trailer.Get("X-Second"))
				})

				t.Run("response_and_trailers", func(t *testing.T) {
					req, _ := http.NewRequestWithContext(t.Context(), "GET", fmt.Sprintf("http://localhost:%d/response-and-trailers", port), nil)
					req.Header.Set("TE", "trailers")
					res, err := cl.Do(req)
					require.NoError(t, err)
					defer res.Body.Close()
					require.Equal(t, http.StatusOK, res.StatusCode)
					body, err := io.ReadAll(res.Body)
					require.NoError(t, err)
					require.Equal(t, "Hello Bear", string(body))
					require.Equal(t, "last", res.Trailer.Get("X-First"))
					require.Equal(t, "first", res.Trailer.Get("X-Second"))
				})
			}

			t.Run("bidi_stream", func(t *testing.T) {
				reqBodyR, reqBodyW := io.Pipe()
				req, _ := http.NewRequestWithContext(t.Context(), "POST", fmt.Sprintf("http://localhost:%d/bidi-stream", port), reqBodyR)
				req.ContentLength = -1
				req.Header.Set("TE", "trailers")
				res, err := cl.Do(req)
				require.NoError(t, err)
				defer res.Body.Close()
				require.Equal(t, http.StatusAccepted, res.StatusCode)
				require.Equal(t, "text/plain", res.Header.Get("Content-Type"))
				require.Equal(t, "bear", res.Header.Get("X-Animal"))

				buf := make([]byte, 1024)
				n, err := res.Body.Read(buf)
				require.NoError(t, err)
				require.Equal(t, "Who are you?", string(buf[:n]))
				_, err = reqBodyW.Write([]byte("Choko"))
				require.NoError(t, err)
				n, err = res.Body.Read(buf)
				require.NoError(t, err)
				require.Equal(t, "Hi Choko. What do you want to do?", string(buf[:n]))
				_, err = reqBodyW.Write([]byte("make money"))
				require.NoError(t, err)
				reqBodyW.Close()
				n, err = res.Body.Read(buf)
				require.NoError(t, err)
				require.Equal(t, "Let's make money!", string(buf[:n]))
				_, err = res.Body.Read(buf)
				require.ErrorIs(t, err, io.EOF)
				if pyinterface == "asgi" {
					require.Equal(t, "great", res.Trailer.Get("X-Result"))
					require.Equal(t, "fast", res.Trailer.Get("X-Time"))
				}
			})
		})
	}
}
