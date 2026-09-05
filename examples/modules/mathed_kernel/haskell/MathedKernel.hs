-- | `mathed_kernel` — the sample hosted module behind mathed's
-- `\kernel` segments (velysterm N11, the australVM plugin-system
-- execution backend).
--
-- A Jupyter kernel is a language runtime trusted after container
-- quarantine; this module is the generalization: the same
-- Jupyter-shaped output contract, but execution is a *granted,
-- audited capability* — deny-by-default (the manifest's `[grants]`
-- uk_* list + the worker's MATHED_EXEC_GRANTS / MATHED_KERNEL_LANGS
-- gates), never per-kernel isolation.
--
-- Wire convention (the same JSON kernel_client uses): read one line
-- of `{"module": ..., "language": ..., "code": ...}` JSON on stdin;
-- write one line of `{"outputs": [<KernelOutput>...]}` JSON on
-- stdout, where KernelOutput mirrors the Jupyter message content:
--
--   {"output_type": "stream", "name": "stdout", "text": "..."}
--   {"output_type": "execute_result", "mime": "text/plain", "data": "..."}
--   {"output_type": "error", "ename": "...", "evalue": "...", "traceback": []}
--
-- v1 answers one stdout stream echoing the code (the compute role is
-- the kernel ops the manifest grants; a real Jupyter kernel attaches
-- through the same op via kernel_client's jupyter_stdio transport).
module MathedKernel where

import System.IO (getLine, isEOF)
import System.Exit (exitSuccess)

-- | JSON-escape a string for the output payload (no aeson dependency:
-- the module compiles in the plain GHC env, mirroring fock_match).
esc :: String -> String
esc = concatMap (\c -> case c of
    '"'  -> "\\\""
    '\\' -> "\\\\"
    '\n' -> "\\n"
    '\r' -> "\\r"
    c    -> [c])

-- | Read the one-line `{code: ...}` payload (empty input = the empty
-- code, which still answers — the contract is total).
readCode :: IO String
readCode = do
    eof <- isEOF
    if eof
        then pure ""
        else getLine

-- | Answer the Jupyter-shaped outputs payload for a code run.
outputsFor :: String -> String
outputsFor code =
    "{\"outputs\":[{\"output_type\":\"stream\",\"name\":\"stdout\","
        ++ "\"text\":\"ran on mathed_kernel: " ++ esc code ++ "\\n\"}]}"

main :: IO ()
main = do
    line <- readCode
    putStrLn (outputsFor line)
    exitSuccess
