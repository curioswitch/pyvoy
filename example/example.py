import asyncio

import httpx

async def main():
    requests = asyncio.Queue()

    async def request_stream():
        try:
            while True:
                req = await requests.get()
                yield req
        except asyncio.QueueShutDown:
            return
    
        
    async with httpx.AsyncClient(http2=True, http1=False) as client:
        async with client.stream("POST", "http://localhost:8000/foo", 
                                content=request_stream()) as res:
            print("got response")
            async for chunk in res.aiter_bytes():
                print(chunk.decode())
                requests.put_nowait(b"hello" + chunk)
                await asyncio.sleep(1)

if __name__ == "__main__":
    asyncio.run(main())