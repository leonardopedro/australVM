{-# LANGUAGE GADTs #-}
{-# LANGUAGE DataKinds #-}
{-# LANGUAGE TypeOperators #-}

module HelloKernel where

import Control.Monad.Freer

data Kernel a where
    Evolve      :: Int64 -> Int64 -> Kernel Int64
    Probability :: Int64 -> Text  -> Kernel Int64
    Condition   :: Int64 -> Text  -> Kernel Int64

evolve :: Member Kernel effs => Int64 -> Int64 -> Eff effs Int64
evolve model steps = send (Evolve model steps)

probability :: Member Kernel effs => Int64 -> Text -> Eff effs Int64
probability model eventJson = send (Probability model eventJson)

condition :: Member Kernel effs => Int64 -> Text -> Eff effs Int64
condition model eventJson = send (Condition model eventJson)

main :: Eff '[Kernel] Int64
main = do
    let model = 1 :: Int64
    rc <- evolve model 10
    prob <- probability model "{\"event\": \"detector_click\"}"
    pure (rc + prob)
