package main

import (
	"io"
	"net/http"
)

func main() {
	reqBodyR, reqBodyW := io.Pipe()
	req, _ := http.NewRequest("POST", "http://localhost:8000/foo", reqBodyR)
	req.ContentLength = -1
	var prots http.Protocols
	prots.SetUnencryptedHTTP2(true)
	cl := &http.Client{
		Transport: &http.Transport{
			ForceAttemptHTTP2: true,
			Protocols:         &prots,
		},
	}
	res, err := cl.Do(req)
	if err != nil {
		panic(err)
	}
	println(res.Proto)
	defer res.Body.Close()
	buf := make([]byte, 1024)
	n, err := res.Body.Read(buf)
	if err != nil {
		panic(err)
	}
	println("response:", string(buf[:n]))
	reqBodyW.Write([]byte("heeeeeeefj;aiejf;aiefj aef;jaefi;jaef;iajef;ieajfa;iejfa;eifjae;ifjaef;jefello"))
	reqBodyW.Close()
	n, err = res.Body.Read(buf)
	if err != nil {
		panic(err)
	}
	println("response:", string(buf[:n]))
}
