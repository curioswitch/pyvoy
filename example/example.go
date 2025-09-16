package main

import (
	"io"
	"net/http"
)

func main() {
	reqBodyR, reqBodyW := io.Pipe()
	req, _ := http.NewRequest("POST", "http://localhost:56438/foo", reqBodyR)
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
	println(res.Status)
	for k, v := range res.Header {
		println("header:", k, v[0])
	}
	buf := make([]byte, 1024)
	n, err := res.Body.Read(buf)
	if err != nil {
		panic(err)
	}
	println("response:", string(buf[:n]))
	println("request: Choko")
	reqBodyW.Write([]byte("Choko"))
	n, err = res.Body.Read(buf)
	if err != nil {
		panic(err)
	}
	println("response:", string(buf[:n]))
	println("request: make money")
	reqBodyW.Write([]byte("make money"))
	reqBodyW.Close()
	n, err = res.Body.Read(buf)
	if err != nil {
		panic(err)
	}
	println("response:", string(buf[:n]))
}
