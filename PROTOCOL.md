
### Blob meta

```
file_name       : String,
file_size       : u64,
block_size      : u32,
block_hash      : [u128; nblocks]
```

### Packet format


opcode | code     | description
-------|--------- | ------------
0      | nop      | No operation. For keep alive connection
1      | hello    | 
2      | ask      | 
3      | ask reply| 

#### Hello

```
proto_version   : u8,
node_id         : u128,

```

# Ask 

```
hash : u128 
```

# Ask Reply

```
packet_size : u32 // < 4MB
```

