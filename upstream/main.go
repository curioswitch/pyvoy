package main

import (
	"log"
	"net"
	"net/http"
	"time"
)

func main() {
	lis, err := net.Listen("tcp", ":0")
	if err != nil {
		log.Fatalf("Unable to open port: %v\n", err)
	}
	var prots http.Protocols
	prots.SetUnencryptedHTTP2(true)
	srv := &http.Server{
		Protocols: &prots,
		Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			w.WriteHeader(200)
			c := http.NewResponseController(w)
			c.Flush()
			<-r.Context().Done()
		}),
		ReadHeaderTimeout: 3 * time.Second,
	}
	log.Printf("Listening on port: %d\n", lis.Addr().(*net.TCPAddr).Port)
	srv.Serve(lis)
}
