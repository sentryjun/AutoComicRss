# SPDX-License-Identifier: GPL-2.0-only
"""Loopback-only SMB2 fixture. Requires impacket==0.13.0 in the selected Python."""
import logging
import sys
from pathlib import Path
from impacket import smbserver, smb3structs
from impacket.nt_errors import STATUS_ACCESS_DENIED
from impacket.ntlm import compute_lmhash, compute_nthash

logging.disable(logging.CRITICAL)
root, ready = sys.argv[1:3]
server = smbserver.SimpleSMBServer(listenAddress='127.0.0.1', listenPort=0)
server.setSMB2Support(True)
server.addShare('MEDIA', root, readOnly='yes')
server.addCredential('reader', 0, compute_lmhash('fixture-password'), compute_nthash('fixture-password'))
# Inject a protocol-level permission refusal independently of the host's UID/ACLs.
original_create = server.getServer().hookSmb2Command(smb3structs.SMB2_CREATE, None)
def create(connection_id, smb_server, packet):
    request = smb3structs.SMB2Create(packet['Data'])
    name = request['Buffer'][:request['NameLength']].decode('utf-16le').replace('\\', '/')
    if 'denied' in name.split('/'):
        return [smb3structs.SMB2Error()], None, STATUS_ACCESS_DENIED
    return original_create(connection_id, smb_server, packet)
server.getServer().hookSmb2Command(smb3structs.SMB2_CREATE, create)
Path(ready).write_text(str(server.getServer().server_address[1]))
server.start()
