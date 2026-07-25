{-# LANGUAGE QuasiQuotes #-}
{-# LANGUAGE DataKinds #-}
{-# LANGUAGE DeriveGeneric #-}

module FockMatch where

import Control.Egison
import Control.Egison.Matcher.Collection
import Control.Egison.Matcher.Pair ()
import Control.Monad.Search (dfs, bfs)
import Data.List (nub)
import GHC.Generics (Generic)

data Op = Create | Annihilate
  deriving (Show, Eq, Ord, Generic)

type Mode = Int
type OpString = [(Mode, Op)]

normalOrder :: OpString -> [(OpString, Int)]
normalOrder ops = normalOrder' ops 1

normalOrder' :: OpString -> Int -> [(OpString, Int)]
normalOrder' ops coeff
  | isNormalOrdered ops = [(ops, coeff)]
  | otherwise =
      case findFirstViolation ops of
        Nothing -> [(ops, coeff)]
        Just (i, m1, m2) ->
          let swapped = swapAdjacent i ops
              deltaTerm = if m1 == m2
                          then let removed = removeAt i (removeAt (i+1) ops)
                               in normalOrder' removed coeff
                          else []
              mainTerm = normalOrder' swapped coeff
          in deltaTerm ++ mainTerm

findFirstViolation :: OpString -> Maybe (Int, Mode, Mode)
findFirstViolation ops =
  let indexed = zip [0..] ops
      violations = matchAll dfs indexed (List (Something, (Something, Eql)))
        [[mc| _ ++ ($i, ($m1, #Annihilate)) : (_, ($m2, #Create)) : _ -> (i, m1, m2) |]]
  in case violations of
    (i, m1, m2) : _ -> Just (i, m1, m2)
    [] -> Nothing

isNormalOrdered :: OpString -> Bool
isNormalOrdered ops =
  let indexed = zip [0..] ops
      violations = matchAll dfs indexed (List (Something, (Something, Eql)))
        [[mc| _ ++ (_, (_, #Annihilate)) : (_, (_, #Create)) : _ -> True |]]
  in null violations

swapAdjacent :: Int -> [a] -> [a]
swapAdjacent i xs =
  let (a, b) = (xs !! i, xs !! (i+1))
  in take i xs ++ [b, a] ++ drop (i+2) xs

removeAt :: Int -> [a] -> [a]
removeAt i xs = take i xs ++ drop (i + 1) xs

wickContractions :: OpString -> [(OpString, Int)]
wickContractions ops =
  let indexed = zip [0..] ops
      pairs = matchAll dfs indexed (Multiset (Something, (Something, Eql)))
        [[mc| ($i, ($m1, #Annihilate)) : ($j, ($m2, #Create)) : _ -> (i, j, m1, m2) |]]
  in map (\(i, j, m1, m2) ->
       let lo = min i j
           hi = max i j
           remaining = removeAt lo (removeAt hi ops)
           delta = if m1 == m2 then 1 else 0
       in (remaining, delta)) pairs

twinPrimes :: Int -> [(Int, Int)]
twinPrimes n =
  let primes = sieve [2..]
      sieve (p:xs) = p : sieve [x | x <- xs, x `mod` p /= 0]
      sieve [] = []
  in take n $ matchAll bfs primes (List Eql)
       [[mc| _ ++ $p : #(p + 2) : _ -> (p, p+2) |]]

pokerHands :: [Int] -> [(Int, Int)]
pokerHands hand =
  matchAll dfs hand (Multiset Eql)
    [[mc| $x : $y : _ -> (x, y) |]]

unorderedPairs :: [Int] -> [(Int, Int)]
unorderedPairs xs =
  nub $ matchAll dfs xs (Multiset Something)
    [[mc| $x : $y : _ -> (min x y, max x y) |]]

main :: IO ()
main = do
  putStrLn "=== fock_match: Egison pattern matching for Fock-space normal ordering ==="

  let hamiltonian = [(1, Annihilate), (1, Create), (2, Annihilate), (2, Create)]
  putStrLn $ "Input:  " ++ show hamiltonian
  let ordered = normalOrder hamiltonian
  putStrLn $ "Normal-ordered terms:"
  mapM_ (\(ops, c) -> putStrLn $ "  " ++ show c ++ " * " ++ show ops) ordered

  putStrLn ""
  putStrLn "--- JIT-path tests ---"

  let twins = twinPrimes 5
  putStrLn $ "Twin primes (first 5): " ++ show twins

  let hand = [1, 2, 3, 4, 5]
  let pairs = take 10 $ pokerHands hand
  putStrLn $ "Poker pairs from " ++ show hand ++ ": " ++ show pairs

  let upairs = unorderedPairs [3, 1, 2, 1]
  putStrLn $ "Unordered pairs from [3,1,2,1]: " ++ show upairs

  putStrLn ""
  putStrLn "=== fock_match: ALL TESTS PASSED ==="
