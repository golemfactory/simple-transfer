

#### Check key

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

#### Check key OK

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

