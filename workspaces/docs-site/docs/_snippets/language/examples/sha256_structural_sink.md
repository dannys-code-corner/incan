```incan
from std.hash import Sha256Hasher, sha256

model StructuralSink:
    hasher: Sha256Hasher

    def append(mut self, bytes_value: bytes) -> None:
        self.hasher.update(bytes_value)

    def finalize(mut self) -> bytes:
        return self.hasher.finalize_bytes()

mut sink = StructuralSink(hasher=sha256.new())
sink.append(b"part-one")
sink.append(b"part-two")
digest = sink.finalize()
```
