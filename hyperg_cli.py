#! /usr/bin/env python3
from typing import List, Optional, Dict, Any, Tuple
from pathlib import Path
from enum import Enum
from json import dumps
import os
import argparse
import requests


class Id:
    """Server identification response representation."""

    def __init__(self, json: Dict[str, Any]) -> None:
        self._node_id: str = json['id']
        self._version: str = json['version']

    def __str__(self) -> str:
        return "id       %s\nversion  %s" % (self.node_id, self.version)

    def __repr__(self) -> str:
        return repr({'node_id': self._node_id, 'version': self._version})

    @property
    def node_id(self) -> str:
        return self._node_id

    @property
    def version(self) -> str:
        return self._version


class Environment(Enum):
    MAINNET = "mainnet"
    TESTNET = "testnet"


class HypergLogContext:

    def __init__(self,
                 node_id: str,
                 env: Environment,
                 node_name: Optional[str] = None,
                 golem_version: Optional[str] = None) -> None:
        self._node_id = node_id if len(
            node_id) < 35 else f"{node_id[:16]}...{node_id[-16:]}"
        self._env = env
        self._node_name = node_name
        self._golem_ver = golem_version

    @property
    def node_id(self) -> str:
        return self._node_id

    @property
    def env(self) -> Environment:
        return self._env

    @property
    def node_name(self) -> Optional[str]:
        return self._node_name

    def golem_version(self) -> Optional[str]:
        return self._golem_ver

    def to_json(self) -> Dict:
        json = {'id': self._node_id, 'env': self.env.value}
        if self._node_name is not None:
            json["nodeName"] = self._node_name
        if self._golem_ver is not None:
            json["golemVersion"] = self._golem_ver

        return json


class HypergClient:

    def __init__(self,
                 rpc_port: Optional[int] = None,
                 log_context: Optional[HypergLogContext] = None) -> None:
        self._url: str = 'http://127.0.0.1:%d/api' % (rpc_port or 3292,)
        self._log_context = log_context

    def _call_rpc(self, json: Dict) -> Dict:
        """
        Sends rpc call to hyperg server.

        :param json: rpc call body.
        :return:
        """

        if self._log_context is not None:
            json = json.copy()
            json["user"] = self._log_context.to_json()

        print(dumps(json))
        response = requests.post(self._url, json=json)
        if response.status_code == 400 or response.status_code == 500:
            print('t=', response.text)
        return response.json()

    def server_id(self) -> Id:
        """RPC call. Returns server version and identyfication data."""
        return Id(self._call_rpc({'command': 'id'}))

    def upload(self, file_names: List[str],
               timeout: Optional[float] = None) -> str:
        """RPC call. Requests files upload. Returns hash of given file set."""
        files = {}
        for name in file_names:
            files[name] = os.path.basename(name)
        res = self._call_rpc({
            'command': 'upload',
            'files': files,
            'timeout': timeout
        })
        return res['hash']

    def download(self, files_hash: str, outdir: Path,
                 peers: List[Tuple[str, int]]):
        """Requests files download from given canditates"""
        result = self._call_rpc({
            'command': 'download',
            'hash': files_hash,
            'dest': str(outdir),
            'peers': [{
                'TCP': peer
            } for peer in peers]
        })
        return result['files']


def parse_addr(addr: str) -> Tuple[str, int]:
    """Parsers addres from <ip>[:<port>] format to tuple with (ip, port)"""
    addr_parts = addr.split(':')
    if len(addr_parts) == 1:
        return (addr_parts[0], 3282)

    return (addr_parts[0], int(addr_parts[1]))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--rpc-port',
                        type=int,
                        help='hyperg rpc port',
                        default=3292)

    log_opts = parser.add_argument_group('sentry', 'sentry logging options')
    log_opts.add_argument('--node-id',
                          help='sentry user id, required for sentry logging')
    log_opts.add_argument('--node-name')
    log_opts.add_argument('--env',
                          type=Environment,
                          nargs='?',
                          help='environment classifier: testnet, mainnet',
                          default=Environment.TESTNET)
    log_opts.add_argument('--golem-version', type=str, nargs='?')

    subparsers = parser.add_subparsers(dest='command')
    _parser_id = subparsers.add_parser('id', help='gets server version')
    parser_upload = subparsers.add_parser('upload', help='requests upload')
    parser_upload.add_argument('file', type=argparse.FileType('r'), nargs='+')
    parser_upload.add_argument('-t',
                               '--timeout',
                               type=float,
                               nargs='?',
                               help='sharing time in seconds')

    parser_download = subparsers.add_parser('download',
                                            help='requests download')
    parser_download.add_argument('hash', type=str, help='file set hash')
    parser_download.add_argument('outdir', type=Path, help='output path')
    parser_download.add_argument('peer',
                                 type=str,
                                 nargs='+',
                                 help='ip[:port] of peer to download from')

    args = parser.parse_args()
    print('=', repr(args))

    user = None
    if args.node_id is not None:
        user = HypergLogContext(args.node_id, args.env, args.node_name,
                                args.golem_version)

    client = HypergClient(rpc_port=args.rpc_port, log_context=user)

    if args.command == 'upload':
        res = client.upload([os.path.abspath(f.name) for f in args.file],
                            args.timeout)
        print('res=', res)
    elif args.command == 'id':
        print(client.server_id())
    elif args.command == 'download':
        outdir = args.outdir
        if not outdir.is_dir():
            outdir.mkdir()

        print(
            client.download(args.hash, outdir,
                            [parse_addr(addr) for addr in args.peer]))
    else:
        parser.print_help()


__all__ = ['HypergClient', 'Id']

if __name__ == '__main__':
    main()
