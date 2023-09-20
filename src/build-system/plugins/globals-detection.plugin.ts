import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { BuildContext } from "../build-context";
import { SourceFile } from "../source-file";
import { BuildPlugin } from "./build-plugin";
import { GameVersion } from "../../util/game-version";
import { Variable, VariableType } from "../variable";

@Injectable()
export class GlobalsDetectionPlugin extends BuildPlugin {

    public parse(buildContext: BuildContext, sourceFile: SourceFile): void {}
    public build(buildContext: BuildContext, gameVersion: GameVersion): void {}
    public ast(sourceFile: SourceFile, node): void {
        switch (node.type) {
            case "FunctionDeclaration":
                this.handleFunctionDeclaration(sourceFile, node)
                break
            case "AssignmentStatement":
                this.handleAssignmentStatement(sourceFile, node)
                break
            case "LocalStatement":
                this.handleAssignmentStatement(sourceFile, node)
                break
        }
    }

    private handleFunctionDeclaration(sourceFile: SourceFile, node): void {
        if (node.identifier) {
            if (node.identifier.name !== undefined) {
                let variable: Variable = new Variable(
                    node.identifier.name,
                    node.identifier.isLocal === false,
                    VariableType.FUNCTION,
                    node.body,
                )
                sourceFile.variables[variable.getFullName()] = variable
                if (node.identifier.isLocal === false) {
                    sourceFile.declaredGlobals.push(variable.getFullName())
                }
            } else {
                let variable: Variable = new Variable(
                    node.identifier.identifier.name,
                    false,
                    VariableType.FUNCTION,
                    node.body,
                )
                sourceFile.variables[variable.getFullName()] = variable
                if (node.identifier.base.isLocal === false) {
                    //sourceFile.declaredGlobals.push(variable.getFullName())
                }
            }
        }
    }

    private handleAssignmentStatement(sourceFile: SourceFile, node): void {
        for (let i = 0; i < node.variables.length; i++) {
            let variable = node.variables[i]
            let value = node.init[i]
            let foundVariable: Variable = new Variable(
                variable.name,
                variable.isLocal === false,
                VariableType.ANY
            ) 
            switch (variable.type) {
                case "Identifier": 
                    if (value !== undefined) {
                        this.evaluateVariableAssignment(sourceFile, foundVariable, value)
                    }
                    break
                case "MemberExpression":
                    foundVariable.name = variable.identifier.name
                    if (value !== undefined) {
                        this.evaluateVariableAssignment(sourceFile, foundVariable, value)
                    }
                    break
                case "IndexExpression":
                    if (variable.base.name === "_G") {
                        foundVariable.global = true
                        this.evaluateVariableAssignment(sourceFile, foundVariable, variable.index)
                        if (foundVariable.value) {
                            foundVariable.name = foundVariable.value.toString()
                            foundVariable.value = undefined
                            this.evaluateVariableAssignment(sourceFile, foundVariable, value)
                        }
                    }
                    break
                default:
                    throw Error("Unknown Variable Type: " + variable.type)
            }
            if (foundVariable.getFullName() !== undefined) {
                sourceFile.variables[foundVariable.getFullName()] = foundVariable
            }
            if (foundVariable.global === true) {
                sourceFile.declaredGlobals.push(foundVariable.getFullName())
            }
        }
    }
    
    private evaluateVariableAssignment(sourceFile: SourceFile, variable: Variable, value): void {
        switch (value.type) {
            case "FunctionDeclaration":
                variable.type = VariableType.FUNCTION
                variable.value = value.body
                break
            case "StringLiteral":
                variable.type = VariableType.STRING
                variable.value = value.raw.replace("\"", "").replace("\"", "")
                break
            case "NumericLiteral":
                variable.type = VariableType.NUMBER
                variable.value = +value.value
                break
            case "BooleanLiteral":
                variable.type = VariableType.BOOLEAN
                variable.value = value.value
                break
            case "NilLiteral":
                variable.type = VariableType.NIL
                variable.value = null
                break
            case "TableConstructorExpression":
                variable.type = VariableType.TABLE
                variable.value = value.fields
                break
            case "BinaryExpression":
                variable.type = VariableType.NUMBER
                variable.value = +value.left.value + +value.right.value
                break
            case "CallExpression":
                // ToDo: Evaluate Return Value of Function
                variable.type = VariableType.ANY
                variable.value = undefined
                break
            case "LogicalExpression":
                variable.type = VariableType.BOOLEAN
                variable.value = undefined
                break
            case "Identifier":
                if (sourceFile.variables[value.name] !== undefined) {
                    variable.type = sourceFile.variables[value.name].type
                    variable.value = sourceFile.variables[value.name].value
                } else {
                    variable.type = VariableType.ANY
                    variable.value = undefined
                }
                break
            case "IndexExpression":
                variable.type = VariableType.ANY
                variable.value = undefined
                break
            case "MemberExpression":
                variable.type = VariableType.ANY
                variable.value = undefined
                break
            case "VarargLiteral":
                variable.type = VariableType.ANY
                variable.value = undefined
                break
            default:
                throw Error("Unknown Variable Asignment: " + value.type)
        }
    }

}