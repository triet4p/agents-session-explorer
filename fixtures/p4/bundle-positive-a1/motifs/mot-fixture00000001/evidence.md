# Evidence — mot-fixture00000001

synthetic: true

## occ-fixture00000001 — selection=earliest
source_alias: src-fixture00000001
input_identity: sha256:6fbd1486b2fd2abd0703aa58d8b5a7b0ae903450a573cb4382feb9dedd243e7d

### call 0
selector: block-v1|span:0:193|message.content[0]|toolCall
block_sha256: sha256:4b10e0578aa882dbae8d30280f27e5b7162c71c2131f6c1f653b462a901e72d8
tool: read
signature: read|path=P
result_status: present
result_selector: block-v1|span:194:217|message.content[0]|toolResult
result_block_sha256: sha256:064f8cb159bee085001353ee686d8c342165b2570e8956af5894d22d0471d315
excerpt:
    call[0] tool=read sig=read|path=P
    result: open «url» then «path»

### call 1
selector: block-v1|span:412:192|message.content[0]|toolCall
block_sha256: sha256:a9d79ebbbcc56ac8703c1eb184cf971487492e0133b148a3a0a920680a2ee23b
tool: read
signature: read|path=P
result_status: present
result_selector: block-v1|span:605:263|message.content[0]|toolResult
result_block_sha256: sha256:e8f186482dbe70613c111acf572f6e305f5939bc2bf11ca874e0c82ff0f6375a
excerpt:
    call[1] tool=read sig=read|path=P
    result: one «path»
            two «email»
            three «secret»

### call 2
selector: block-v1|span:869:193|message.content[0]|toolCall
block_sha256: sha256:fda760d614b187150773098e8e7aba35914cd02bba2304b68ff43e40c486868e
tool: read
signature: read|path=P
result_status: present
result_selector: block-v1|span:1063:216|message.content[0]|toolResult
result_block_sha256: sha256:d22b835292757cb62c81d6c70c673d4bf04998ecc87721dc2f4f9716af124201
excerpt:
    call[2] tool=read sig=read|path=P
    result: blob «token» end

## occ-fixture00000002 — selection=latest
source_alias: src-fixture00000001
input_identity: sha256:f40c1a414924e754bc45459045ebd098df58cab4e86e14ca9461a7c4760c77c2

### call 0
selector: block-v1|span:1280:193|message.content[0]|toolCall
block_sha256: sha256:a4f54d3a39c8f480fda49f9425c5eb4b00fef236da671a2526dae5a3e87db862
tool: read
signature: read|path=P
result_status: present
result_selector: block-v1|span:1474:179|message.content[0]|toolResult
result_block_sha256: sha256:0102256383f046943fe636bb7ee409c772276baab8abfaab0f55677be0ac1f02
excerpt:
    call[0] tool=read sig=read|path=P
    result: delta «path»

### call 1
selector: block-v1|span:1654:195|message.content[0]|toolCall
block_sha256: sha256:826fe6a5a5ced83d024145e13d7fcac6b62f0c7c7bb231a13d7ad2b658b87ec6
tool: read
signature: read|path=P
result_status: present
result_selector: block-v1|span:1850:169|message.content[0]|toolResult
result_block_sha256: sha256:2ee9a51570123e781b951b3501e2c8dab5532840046ab2f650654809c281b926
excerpt:
    call[1] tool=read sig=read|path=P
    result: epsilon ok

### call 2
selector: block-v1|span:2020:193|message.content[0]|toolCall
block_sha256: sha256:c3d17a66dd8674d6a8ebb1cf2d4f285bc670a785c18421c24622b5452b650a2c
tool: read
signature: read|path=P
result_status: present
result_selector: block-v1|span:2214:167|message.content[0]|toolResult
result_block_sha256: sha256:4f9890165cb6669bc8182c290db64453699025611bc0fbf778141725c6921e09
excerpt:
    call[2] tool=read sig=read|path=P
    result: zeta ok

