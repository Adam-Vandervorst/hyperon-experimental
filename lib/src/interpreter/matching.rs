use super::*;
use crate::*;
use crate::matcher::*;

use std::rc::Rc;

// TODO: error handing and logging can also be moved into Plan structures
// or implemented atop of them
type InterpreterResult = Result<Vec<(Atom, Bindings)>, String>;

fn merge_results(plan_res: InterpreterResult, step_res: InterpreterResult) -> InterpreterResult {
    match (step_res, plan_res) {
        (Ok(mut step_res), Ok(mut plan_res)) => {
            plan_res.append(&mut step_res);
            Ok(plan_res)
        },
        (_, Err(msg)) => Err(format!("Unexpected error: {}", msg)),
        (Err(_), plan_res) => plan_res,
    }
}

pub fn interpret(space: Rc<GroundingSpace>, expr: &Atom) -> Result<Vec<Atom>, String> {
    execute_plan(interpret_op, (space, expr.clone(), Bindings::new()))
        .map(|mut res| res.drain(0..).map(|(atom, _)| atom).collect())
}

fn is_grounded(expr: &ExpressionAtom) -> bool {
    matches!(expr.children().get(0), Some(Atom::Grounded(_)))
}

fn interpret_op((space, atom, bindings): (Rc<GroundingSpace>, Atom, Bindings)) -> StepResult<(), InterpreterResult> {
    let atom = apply_bindings_to_atom(&atom, &bindings);
    log::debug!("interpret_op: {}", atom);
    if let Atom::Expression(ref expr) = atom {
        if expr.is_plain() {
            StepResult::execute(ApplyPlan::new(interpret_reducted_op, (space,  atom, bindings)))
        } else {
            if is_grounded(expr) {
                StepResult::execute(ApplyPlan::new(reduct_op, (space,  atom, bindings)))
            } else {
                StepResult::execute(SequencePlan::new(
                        ApplyPlan::new(match_op, (Rc::clone(&space), atom.clone(), bindings.clone())),
                        PartialApplyPlan::new(reduct_first_if_not_matched_op, (space, atom, bindings))
                ))
            }
        }
    } else {
        StepResult::ret(Ok(vec![(atom, bindings)]))
    }
}

fn interpret_reducted_op((space, atom, bindings): (Rc<GroundingSpace>, Atom, Bindings)) -> StepResult<(), InterpreterResult> {
    let atom = apply_bindings_to_atom(&atom, &bindings);
    log::debug!("interpret_reducted_op: {}", atom);
    if let Atom::Expression(ref expr) = atom {
        if is_grounded(expr) {
            StepResult::execute(ApplyPlan::new(execute_op, (atom, bindings)))
        } else {
            StepResult::execute(SequencePlan::new(
                    ApplyPlan::new(match_op, (space, atom.clone(), bindings.clone())),
                    PartialApplyPlan::new(return_if_not_matched_op, (atom, bindings))
            ))
        }
    } else {
        StepResult::ret(Ok(vec![(atom, bindings)]))
    }
}

fn return_if_not_matched_op((default, match_result): ((Atom, Bindings), InterpreterResult)) -> StepResult<(), InterpreterResult> {
    StepResult::ret(match_result.map(
            |vec| if vec.is_empty() { vec![default] } else { vec }))
}

fn reduct_first_if_not_matched_op(((space, expr, bindings), match_result): ((Rc<GroundingSpace>, Atom, Bindings), InterpreterResult)) -> StepResult<(), InterpreterResult> {
    match match_result {
        Ok(vec) if vec.is_empty() => {
            StepResult::execute(ApplyPlan::new(reduct_first_op, (space, expr, bindings)))
        },
        _ => StepResult::ret(match_result),
    }
}

fn reduct_first_op((space, expr, bindings): (Rc<GroundingSpace>, Atom, Bindings)) -> StepResult<(), InterpreterResult> {
    log::debug!("reduct_first_op: {}", expr);
    if let Atom::Expression(expr) = expr {
        let mut iter = BottomSubexprStream::from(expr);
        let sub;
        {
            iter.next();
            sub = iter.get_mut().clone();
        }
        StepResult::execute(SequencePlan::new(
                ApplyPlan::new(interpret_reducted_op, (Rc::clone(&space), sub, bindings)),
                PartialApplyPlan::new(interpret_if_reducted_op, (space, iter))
        ))
    } else {
        StepResult::ret(Err(format!("Atom::Expression is expected as an argument, found: {}", expr)))
    }
}

fn interpret_if_reducted_op(((space, mut iter), reduction_result): ((Rc<GroundingSpace>, BottomSubexprStream), InterpreterResult)) -> StepResult<(), InterpreterResult> {
    log::debug!("interpret_if_reducted_op: reduction_result: {:?}", reduction_result);
    match reduction_result {
        Err(_) => StepResult::ret(reduction_result),
        Ok(mut vec) if vec.len() == 1 && vec[0].0 == *(iter.get_mut()) => {
            let (result, bindings) = vec.pop().unwrap();
            if iter.has_next() {
                let next_sub;
                {
                    iter.next();
                    next_sub = iter.get_mut().clone();
                    log::debug!("interpret_if_reducted_op: reduct next_sub: {}", next_sub);
                }
                StepResult::execute(SequencePlan::new(
                        ApplyPlan::new(interpret_reducted_op, (Rc::clone(&space), next_sub, bindings)),
                        PartialApplyPlan::new(interpret_if_reducted_op, (space, iter))
                ))
            } else {
                StepResult::ret(Ok(vec![(result, bindings)]))
            }
        },
        Ok(mut vec) => {
                let plan = vec.drain(0..)
                    .into_parallel_plan(Ok(vec![]),
                        |(result, bindings)| {
                            let mut iter = iter.clone();
                            if iter.has_next() {
                                *iter.get_mut() = result;
                                Box::new(ApplyPlan::new(interpret_op, (Rc::clone(&space), iter.into_atom(), bindings)))
                            } else {
                                Box::new(|_:()| StepResult::ret(Ok(vec![(result, bindings)])))
                            }
                        },
                        merge_results);
                StepResult::Execute(plan)
        },
    }
}

fn reduct_op((space, expr, bindings): (Rc<GroundingSpace>, Atom, Bindings)) -> StepResult<(), InterpreterResult> {
    log::debug!("reduct_op: {}", expr);
    if let Atom::Expression(expr) = expr {
        let mut iter = BottomSubexprStream::from(expr);
        let sub;
        {
            iter.next();
            sub = iter.get_mut().clone();
        }
        StepResult::execute(SequencePlan::new(
                ApplyPlan::new(interpret_reducted_op, (Rc::clone(&space), sub, bindings)),
                PartialApplyPlan::new(reduct_next_op, (space, iter))
        ))
    } else {
        StepResult::ret(Err(format!("Atom::Expression is expected as an argument, found: {}", expr)))
    }
}

fn reduct_next_op(((space, iter), prev_result): ((Rc<GroundingSpace>, BottomSubexprStream), InterpreterResult)) -> StepResult<(), InterpreterResult> {
    match prev_result {
        Err(_) => StepResult::ret(prev_result),
        Ok(mut results) => {
            let plan = results.drain(0..)
                .map(|(reducted, bindings)| {
                    log::debug!("reduct_next: reducted: {}, bindings: {:?}", reducted, bindings);
                    let mut iter = iter.clone();
                    let next_sub;
                    {
                        *iter.get_mut() = reducted;
                        iter.next();
                        next_sub = iter.get_mut().clone();
                        log::debug!("reduct_next: expression: {}", iter.as_atom());
                        log::debug!("reduct_next: next_sub after reduction: {}", next_sub);
                    }
                    (next_sub, bindings, iter)
                })
                .into_parallel_plan(Ok(vec![]),
                    |(next_sub, bindings, iter)| {
                        if iter.has_next() {
                            Box::new(SequencePlan::new(
                                    ApplyPlan::new(interpret_op, (Rc::clone(&space), next_sub, bindings)),
                                    PartialApplyPlan::new(reduct_next_op, (Rc::clone(&space), iter))
                            ))
                        } else {
                            Box::new(ApplyPlan::new(interpret_reducted_op, (Rc::clone(&space), next_sub, bindings)))
                        }
                    },
                    merge_results);
            StepResult::Execute(plan)
        },
    }
}

fn execute_op((atom, bindings): (Atom, Bindings)) -> StepResult<(), InterpreterResult> {
    log::debug!("execute_op: {}", atom);
    if let Atom::Expression(mut expr) = atom {
        let op = expr.children().get(0).cloned();
        if let Some(Atom::Grounded(op)) = op {
            // TODO: change API, remove boilerplate
            let mut ops: Vec<Atom> = Vec::new();
            let mut data :Vec<Atom> = Vec::new();
            expr.children_mut().drain(1..).into_iter().rev().for_each(|atom| data.push(atom));
            match op.execute(&mut ops, &mut data) {
                Ok(()) => StepResult::ret(Ok(data.drain(0..).map(|atom| (atom, bindings.clone())).collect())),
                Err(msg) => StepResult::ret(Err(msg)),
            }
        } else {
            StepResult::ret(Err(format!("Trying to execute non grounded atom: {}", expr)))
        }
    } else {
        StepResult::ret(Err(format!("Unexpected non expression argument: {}", atom)))
    }
}

fn match_op((space, expr, prev_bindings): (Rc<GroundingSpace>, Atom, Bindings)) -> StepResult<(), InterpreterResult> {
    log::debug!("match_op: {}", expr);
    let var_x = VariableAtom::from("X");
    // TODO: unique variable?
    let atom_x = Atom::Variable(var_x.clone());
    let mut local_bindings = space.query(&Atom::expr(&[Atom::sym("="), expr.clone(), atom_x]));
    if local_bindings.is_empty() {
        log::debug!("match_op: no matches found");
        StepResult::ret(Ok(vec![]))
    } else {
        let plan = local_bindings
            .drain(0..)
            .map(|mut binding| {
                let result = binding.get(&var_x).unwrap(); 
                let result = apply_bindings_to_atom(result, &binding);
                let bindings = apply_bindings_to_bindings(&binding, &prev_bindings);
                let bindings = bindings.map(|mut bindings| {
                    binding.drain().for_each(|(k, v)| { bindings.insert(k, v); });
                    bindings
                });
                log::debug!("match_op: query: {}, binding: {:?}, result: {}", expr, bindings, result);
                (result, bindings)
            })
            .filter(|(_, bindings)| bindings.is_ok())
            .map(|(result, bindings)| (result, bindings.unwrap()))
            .into_parallel_plan(Ok(vec![]),
                |(result, bindings)| Box::new(
                    ApplyPlan::new(interpret_op, (Rc::clone(&space), result, bindings))),
                merge_results);
        StepResult::Execute(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    
    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_match_all() {
        init();
        let mut space = GroundingSpace::new();
        space.add(expr!("=", ("color"), "blue"));
        space.add(expr!("=", ("color"), "red"));
        space.add(expr!("=", ("color"), "green"));
        let expr = expr!(("color"));

        assert_eq!(interpret(Rc::new(space), &expr),
            Ok(vec![expr!("blue"), expr!("red"), expr!("green")]));
    }

    #[test]
    fn test_frog_reasoning() {
        init();
        let mut space = GroundingSpace::new();
        space.add(expr!("=", ("and", "True", "True"), "True"));
        space.add(expr!("=", ("if", "True", then, else), then));
        space.add(expr!("=", ("if", "False", then, else), else));
        space.add(expr!("=", ("Fritz", "croaks"), "True"));
        space.add(expr!("=", ("Fritz", "eats-flies"), "True"));
        space.add(expr!("=", ("Tweety", "chirps"), "True"));
        space.add(expr!("=", ("Tweety", "yellow"), "True"));
        space.add(expr!("=", ("Tweety", "eats-flies"), "True"));
        let expr = expr!("if", ("and", (x, "croaks"), (x, "eats-flies")),
            ("=", (x, "frog"), "True"), "nop");

        assert_eq!(interpret(Rc::new(space), &expr),
            Ok(vec![expr!("=", ("Fritz", "frog"), "True")]));
    }

    #[test]
    fn test_variable_keeps_value_in_different_sub_expressions() {
        init();
        let mut space = GroundingSpace::new();
        space.add(expr!("=", ("eq", x, x), "True"));
        space.add(expr!("=", ("plus", "Z", y), y));
        space.add(expr!("=", ("plus", ("S", k), y), ("S", ("plus", k, y))));
        let space = Rc::new(space);

        assert_eq!(interpret(space.clone(), &expr!("eq", ("plus", "Z", n), n)),
            Ok(vec![expr!("True")]));
        assert_eq!(interpret(space.clone(), &expr!("eq", ("plus", ("S", "Z"), n), n)),
            Ok(vec![expr!("eq", ("S", y), y)]));
    }
}

