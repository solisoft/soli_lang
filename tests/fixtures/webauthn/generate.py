"""Regenerate tests/fixtures/webauthn/vectors.json.

WebAuthn registration and authentication responses for an ES256, an RS256 and
an EdDSA (Ed25519) credential, plus "packed" self-attested registrations,
signed by the OpenSSL CLI rather than by the Rust code under test, so the
vectors are an independent implementation of the signing side.

    python3 tests/fixtures/webauthn/generate.py [workdir]

Fresh keys are generated into workdir (a temp directory by default) and are
not kept: only the public halves end up in the vectors. rp_id is example.com
and the origin https://example.com.
"""
import json, hashlib, base64, subprocess, struct, os, sys, tempfile
D=sys.argv[1] if len(sys.argv)>1 else tempfile.mkdtemp()
OUT=os.path.join(os.path.dirname(os.path.abspath(__file__)),'vectors.json')
def b64u(b): return base64.urlsafe_b64encode(b).rstrip(b'=').decode()
def run(*a, inp=None): return subprocess.run(a, input=inp, capture_output=True, check=True).stdout
# keys
ec=os.path.join(D,'ec.pem'); rsa=os.path.join(D,'rsa.pem'); ed=os.path.join(D,'ed.pem')
if not os.path.exists(ec): run('openssl','ecparam','-name','prime256v1','-genkey','-noout','-out',ec)
if not os.path.exists(rsa): run('openssl','genpkey','-algorithm','RSA','-pkeyopt','rsa_keygen_bits:2048','-out',rsa)
if not os.path.exists(ed): run('openssl','genpkey','-algorithm','ed25519','-out',ed)
edpub=run('openssl','pkey','-in',ed,'-pubout','-outform','DER')[-32:]
ecpub=run('openssl','ec','-in',ec,'-pubout','-outform','DER'); pt=ecpub[-65:]; assert pt[0]==4
x,y=pt[1:33],pt[33:]
mod=run('openssl','rsa','-in',rsa,'-modulus','-noout').decode().strip().split('=')[1]
n=bytes.fromhex(mod); e=(65537).to_bytes(3,'big')
# cbor encoder
def head(mt,v):
    if v<24: return bytes([mt<<5|v])
    if v<256: return bytes([mt<<5|24,v])
    if v<65536: return bytes([mt<<5|25])+v.to_bytes(2,'big')
    return bytes([mt<<5|26])+v.to_bytes(4,'big')
def cbor(o):
    if isinstance(o,bool): return bytes([0xf5 if o else 0xf4])
    if isinstance(o,int): return head(0,o) if o>=0 else head(1,-1-o)
    if isinstance(o,bytes): return head(2,len(o))+o
    if isinstance(o,str): b=o.encode(); return head(3,len(b))+b
    if isinstance(o,list): return head(4,len(o))+b''.join(cbor(i) for i in o)
    if isinstance(o,dict): return head(5,len(o))+b''.join(cbor(k)+cbor(v) for k,v in o.items())
    raise TypeError(o)
RP='example.com'; ORIGIN='https://example.com'
rph=hashlib.sha256(RP.encode()).digest()
AAGUID=bytes.fromhex('adce000235bcc60a648b0b25f1f05503')
def sign(key,msg):
    if key==ed:
        # Ed25519 signs the message itself (PureEdDSA), no prehash.
        m=os.path.join(D,'msg.bin'); open(m,'wb').write(msg)
        return run('openssl','pkeyutl','-sign','-inkey',key,'-rawin','-in',m)
    return run('openssl','dgst','-sha256','-sign',key,inp=msg)
def cdj(typ,chal): return json.dumps({"type":typ,"challenge":chal,"origin":ORIGIN,"crossOrigin":False},separators=(',',':')).encode()
out={}
CREDS=[('es256',ec,-7,{1:2,3:-7,-1:1,-2:x,-3:y}),('rs256',rsa,-257,{1:3,3:-257,-1:n,-2:e}),('eddsa',ed,-8,{1:1,3:-8,-1:6,-2:edpub})]
for name,key,alg,cose in CREDS:
    credid=hashlib.sha256(name.encode()).digest()[:16]
    rchal=b64u(hashlib.sha256(b'reg-'+name.encode()).digest())
    achal=b64u(hashlib.sha256(b'auth-'+name.encode()).digest())
    flags=0x01|0x04|0x08|0x40  # UP UV BE AT
    ad=rph+bytes([flags])+struct.pack('>I',0)+AAGUID+struct.pack('>H',len(credid))+credid+cbor(cose)
    att=cbor({"fmt":"none","attStmt":{},"authData":ad})
    rc=cdj("webauthn.create",rchal)
    reg={"id":b64u(credid),"rawId":b64u(credid),"type":"public-key","response":{"clientDataJSON":b64u(rc),"attestationObject":b64u(att),"transports":["internal","hybrid"]},"clientExtensionResults":{}}
    aflags=0x01|0x04|0x08
    aad=rph+bytes([aflags])+struct.pack('>I',1)
    ac=cdj("webauthn.get",achal)
    sig=sign(key,aad+hashlib.sha256(ac).digest())
    auth={"id":b64u(credid),"rawId":b64u(credid),"type":"public-key","response":{"clientDataJSON":b64u(ac),"authenticatorData":b64u(aad),"signature":b64u(sig),"userHandle":b64u(b'user-1')},"clientExtensionResults":{}}
    # The same registration, "packed" self-attested: attStmt {alg, sig}, the
    # signature by the credential key over authData || SHA-256(clientDataJSON).
    psig=sign(key,ad+hashlib.sha256(rc).digest())
    patt=cbor({"fmt":"packed","attStmt":{"alg":alg,"sig":psig},"authData":ad})
    preg=json.loads(json.dumps(reg)); preg["response"]["attestationObject"]=b64u(patt)
    out[name]={"registration":reg,"packed_registration":preg,"registration_challenge":rchal,"authentication":auth,"authentication_challenge":achal,"public_key":b64u(cbor(cose)),"credential_id":b64u(credid)}
json.dump(out,open(OUT,'w'),indent=1)
open(OUT,'a').write('\n')
print("ok")
