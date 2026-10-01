let rec weight (l: (int) list) : int =
  match l with
  | [] -> 0
  | x :: tl -> x + weight tl

let rec fuel (l: (int) list) : int =
  match l with
  | [] -> 0
  | _ :: tl -> 1 + fuel tl

