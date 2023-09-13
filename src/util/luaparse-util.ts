export interface VariableAssignment {
    type: string,
    target: string,
    index?: string,
    indexLookupType?:"CallExpression" | "Identifier" | "StringLiteral" | "NumericLiteral",
    valueLookupType?: "CallExpression" | "Identifier" | "StringLiteral" | "NumericLiteral",
    value?: any
}

export function getVariableAssignments(node) {
    const assignments = [];
    if (node.body) {
        for (const statement of node.body) {
            assignments.push(...getVariableAssignments(statement));
        }
    }
    if (node.type === "AssignmentStatement" || node.type === "LocalStatement") {
        for (let i = 0; i < node.variables.length; i++) {
            let variable = node.variables[i];
            let init = node.init[i];
            switch (variable.type) {
                case "Identifier": {
                    if (init) {
                        let assignment: VariableAssignment = {
                            type: variable.type,
                            target: variable.name,
                            valueLookupType: init.type,
                        }
                        switch (init.type) {
                            case "CallExpression":
                                assignment.value = init.base.name
                                break
                            case "Identifier":
                                assignment.value = init.name
                                break
                            case "NumericLiteral":
                                assignment.value = +init.raw
                                break
                            case "StringLiteral":
                                assignment.value = init.raw
                                break
                        }
                        assignments.push(assignment)
                    }
                    break
                }
                case "IndexExpression": {
                    let assignment: VariableAssignment = {
                        type: variable.type,
                        target: variable.base.name,
                        valueLookupType: init.type,
                        indexLookupType: variable.index.type,
                    }
                    switch (variable.index.type) {
                        case "Identifier":
                            assignment.index = variable.index.name
                            break
                        case "StringLiteral" || "NumericLiteral":
                            assignment.index = variable.index.raw
                            break
                        case "CallExpression":
                            assignment.index = variable.index.base.name
                            break
                    }
                    switch (init.type) {
                        case "CallExpression":
                            assignment.value = init.base.name
                            break
                        case "Identifier":
                            assignment.value = init.name
                            break
                        case "NumericLiteral":
                            assignment.value = +init.raw
                            break
                        case "StringLiteral":
                            assignment.value = init.raw
                            break
                    }
                    assignments.push(assignment)    
                    break
                }
            }
        }
    }
    return assignments;
}