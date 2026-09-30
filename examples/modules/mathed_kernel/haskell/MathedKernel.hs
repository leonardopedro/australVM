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
import Data.Char (isSpace)

-- | JSON-escape a string for the output payload (no aeson dependency:
-- the module compiles in the plain GHC env, mirroring fock_match).
esc :: String -> String
esc = concatMap (\c -> case c of
    '"'  -> "\\\""
    '\\' -> "\\\\"
    '\n' -> "\\n"
    '\r' -> "\\r"
    c    -> [c])

-- | Decode one JSON string whose opening quote is the first char of
-- the input: returns (decoded, rest after the closing quote). The
-- sample only needs the escapes its own `esc` emits plus the common
-- ones; anything else passes through literally.
decodeString :: String -> (String, String)
decodeString ('"' : s) = go s
  where
    go [] = ([], [])
    go ('\\' : 'n' : t)  = let (r, rest) = go t in ('\n' : r, rest)
    go ('\\' : 'r' : t)  = let (r, rest) = go t in ('\r' : r, rest)
    go ('\\' : 't' : t)  = let (r, rest) = go t in ('\t' : r, rest)
    go ('\\' : '/' : t)  = let (r, rest) = go t in ('/' : r, rest)
    go ('\\' : '\\' : t) = let (r, rest) = go t in ('\\' : r, rest)
    go ('\\' : '"' : t)  = let (r, rest) = go t in ('"' : r, rest)
    go ('"' : t)          = ([], t)
    go (c : t)            = let (r, rest) = go t in (c : r, rest)
decodeString s = ([], s)

-- | The `code` field's value from the one-line
-- `{"module": ..., "language": ..., "code": ...}` envelope (v1
-- values are strings). Scanning skips the earlier string fields so a
-- stray `"code"` inside a value can never be mistaken for the key.
codeOf :: String -> String
codeOf line = fields (dropWhile isSpace line)
  where
    fields ('{' : s) = nextKey (dropWhile isSpace s)
    fields _ = ""
    nextKey ('"' : t) =
        let (key, rest1) = decodeString ('"' : t)
            rest2 = dropWhile isSpace rest1
        in case rest2 of
             (':' : v) -> value key (dropWhile isSpace v)
             _         -> ""
    nextKey _ = ""
    value key v
        | key == "code" = case v of
            ('"' : _) -> fst (decodeString v)
            _         -> ""
        | otherwise = case v of
            -- Skip this field's string value, then continue after the comma.
            ('"' : _) -> afterValue (snd (decodeString v))
            _         -> ""
    afterValue s = case dropWhile isSpace s of
        (',' : t) -> nextKey (dropWhile isSpace t)
        _         -> ""

-- | Read the one-line `{module, language, code}` payload and pull out
-- the code (empty/malformed input = the empty code, which still
-- answers — the contract is total).
readCode :: IO String
readCode = do
    eof <- isEOF
    if eof
        then pure ""
        else codeOf <$> getLine

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
