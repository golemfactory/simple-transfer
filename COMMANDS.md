### (1) Version check

```
POST /api HTTP/1.1

{"command": "id"}
```

```
{"id":"a69ca2ee780ce5df57855982dc6cb37e3cec6e408c2dcb54750bfafaf4fb13a2","version":"0.2.6"}
```

### (2) Addresses

```
POST /api

{"command": "addresses"}
```

```
{"addresses":{"TCP":{"address":"0.0.0.0","port":3282}}}
```


### (3) Adding file

```
POST /api

{"command": "upload", "id": null, "files": {"/home/prekucki/.local/share/golem/default/rinkeby/ComputerRes/e339a264-71a9-11e9-b4e5-b6178fcd50f4/resources/e339a264-71a9-11e9-b4e5-b6178fcd50f4": "e339a264-71a9-11e9-b4e5-b6178fcd50f4"}, "timeout": null}
```

```
{"hash":"f88a92ddbadcfe23e976d92ba5019a81e5d818df4609adc01330d753834c46d8"}
```

### Download

```
POST /api HTTP/1.1

{"command": "download", "hash": "c0ceff522b00eccb95c43b43af67c9585c3d914642339f770800dd164d8b42cc", "dest": "/home/prekucki/.local/share/golem/default/rinkeby/ComputerRes/nonce/tmp", "peers": [{"TCP": ["10.30.10.219", 3282]}, {"TCP": ["10.30.10.219", 3282]}, {"TCP": ["5.226.70.53", 3282]}, {"TCP": ["172.17.0.1", 3282]}], "size": null, "timeout": null}
```

```
{"files":["/home/prekucki/.local/share/golem/default/rinkeby/ComputerRes/nonce/tmp/2047c8a0-fb9e-4306-a116-0df79367bd9e"]}
```


### Check key

```
POST /api HTTP/1.1
Host: localhost:3292
content-type: application/json


{
  "command": "upload", 
   "id": null, 
   "hash": "9854003e54f053e2d911b67bcbf38260", 
   "timeout": null
}
```

```
400 Bad request

{
  "error":"NotFoundError: Key not found in database [6af750676216564288a098837ad99e965c9e4e8c7df610e1e2095d597c95d3b0]"
}
``` 

### Check key OK

```
POST /api HTTP/1.1
Host: localhost:3292
content-type: application/json


{
    "command": "upload", 
    "id": null,
    "hash": "612dd6a00e0e5cd784bdae7de99c78de9bebd7fabd8dfab70adfe2b607c8eccc", 
    "timeout": 234
 }


```

```
HTTP/1.1 200 OK
Content-Type: application/json

{
    "hash":"612dd6a00e0e5cd784bdae7de99c78de9bebd7fabd8dfab70adfe2b607c8eccc"
}
```

