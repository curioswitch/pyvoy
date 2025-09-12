package main

import (
	"net/http"
	"time"
)

func main() {
	var prots http.Protocols
	prots.SetUnencryptedHTTP2(true)
	srv := &http.Server{
		Addr:      ":8002",
		Protocols: &prots,
		Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			println("Got req")
			w.WriteHeader(200)
			c := http.NewResponseController(w)
			c.EnableFullDuplex()
			w.Write([]byte("a"))
			c.Flush()
			<-r.Context().Done()
		}),
		ReadHeaderTimeout: 3 * time.Second,
	}
	srv.ListenAndServe()
}
