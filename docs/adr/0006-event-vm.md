# Original event VM subset

The first legacy VM uses source-audited opcode values and semantics for `NOP`, signed relative jumps, conditional jumps, expressions, `return`, timed `Wait`, synchronous/asynchronous windows, `WaitWindow`, `CloseWindow`, and actor movement.

The expression interpreter uses the original constant, variable, arithmetic, comparison, assignment, and expression-end opcode values. Variable writes preserve the declared integer width and signedness. Division or remainder by zero follows the audited reference behavior of retaining the left operand.

Each entry owns its program counter, variables, wait state, and lifecycle. A field module steps entries in stable ID order. Unsupported opcodes, malformed operands, host failures, and instruction-budget exhaustion report the byte offset and operation. No real FFIX field is claimed compatible until its actual event archive and host dependencies pass.
